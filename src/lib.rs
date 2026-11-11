//! PocketCards — NEP-171 NFTs that hold NEAR and spend it by rules.
//!
//! Each card is a standard NFT that carries its own NEAR balance plus a spending
//! policy chosen by the issuer at mint time:
//!
//! * `daily_limit`: max yoctoNEAR the holder can spend per rolling 24h window (0 = no cap)
//! * `allowlist`: optional list of accounts the card may pay (e.g. merchants, APIs)
//! * `expires_at`: optional expiry; after it the card can't spend and the issuer can reclaim the rest
//!
//! Whoever holds the NFT can spend from it within those rules. Transferring the NFT
//! (nft_transfer / any NEP-171 wallet) hands over the money *and* the rules together.
//! The card art (`metadata.media`) is an on-chain SVG regenerated on every view, so
//! wallets always show the live balance.

use near_contract_standards::non_fungible_token::core::{
    NonFungibleTokenCore, NonFungibleTokenResolver,
};
use near_contract_standards::non_fungible_token::enumeration::NonFungibleTokenEnumeration;
use near_contract_standards::non_fungible_token::metadata::{
    NFT_METADATA_SPEC, NFTContractMetadata, NonFungibleTokenMetadataProvider, TokenMetadata,
};
use near_contract_standards::non_fungible_token::{NonFungibleToken, Token, TokenId};
use near_sdk::base64::Engine;
use near_sdk::collections::LazyOption;
use near_sdk::json_types::{U64, U128};
use near_sdk::serde_json::json;
use near_sdk::store::LookupMap;
use near_sdk::{
    AccountId, BorshStorageKey, Gas, NearToken, PanicOnDefault, Promise, PromiseError,
    PromiseOrValue, env, log, near, require,
};
use std::collections::HashMap;

pub const DAY_NS: u64 = 24 * 60 * 60 * 1_000_000_000;
/// Extra bytes reserved at mint so later state growth (last_spend, counters) is prepaid.
pub const STORAGE_RESERVE_BYTES: u64 = 256;
pub const MAX_ALLOWLIST: usize = 10;
pub const MAX_TITLE_LEN: usize = 48;
const GAS_FOR_CALLBACK: Gas = Gas::from_tgas(10);

#[derive(BorshStorageKey)]
#[near]
enum StorageKey {
    NonFungibleToken,
    Metadata,
    TokenMetadata,
    Enumeration,
    Cards,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct LastSpend {
    pub to: AccountId,
    pub amount: U128,
    pub at_ms: U64,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct Card {
    pub issuer: AccountId,
    pub title: String,
    pub balance: u128,
    pub daily_limit: u128,
    pub allowlist: Option<Vec<AccountId>>,
    pub expires_at_ns: Option<u64>,
    pub created_at_ns: u64,
    pub window_start_ns: u64,
    pub spent_in_window: u128,
    pub total_funded: u128,
    pub total_spent: u128,
    pub spend_count: u64,
    pub last_spend: Option<LastSpend>,
    pub reclaimed: bool,
}

/// JSON view of a card, as returned by `get_card` / `cards_for_owner`.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq)]
pub struct CardView {
    pub token_id: TokenId,
    pub owner_id: AccountId,
    pub issuer: AccountId,
    pub title: String,
    pub balance: U128,
    pub daily_limit: U128,
    pub spent_today: U128,
    pub available_now: U128,
    pub window_resets_at_ms: U64,
    pub allowlist: Option<Vec<AccountId>>,
    pub expires_at_ms: Option<U64>,
    pub is_expired: bool,
    pub created_at_ms: U64,
    pub total_funded: U128,
    pub total_spent: U128,
    pub spend_count: U64,
    pub last_spend: Option<LastSpend>,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Contract {
    tokens: NonFungibleToken,
    metadata: LazyOption<NFTContractMetadata>,
    cards: LookupMap<TokenId, Card>,
    next_id: u64,
}

impl Card {
    fn is_expired(&self, now: u64) -> bool {
        self.expires_at_ns.is_some_and(|e| now >= e)
    }
    /// (spent_in_current_window, window_start) after rolling the window forward if needed.
    fn window(&self, now: u64) -> (u128, u64) {
        if now >= self.window_start_ns.saturating_add(DAY_NS) {
            (0, now)
        } else {
            (self.spent_in_window, self.window_start_ns)
        }
    }
    fn available_now(&self, now: u64) -> u128 {
        if self.is_expired(now) || self.reclaimed {
            return 0;
        }
        if self.daily_limit == 0 {
            return self.balance;
        }
        let (spent, _) = self.window(now);
        self.daily_limit.saturating_sub(spent).min(self.balance)
    }
}

fn emit(event: &str, data: near_sdk::serde_json::Value) {
    log!(
        "EVENT_JSON:{}",
        json!({"standard": "pocketcards", "version": "1.0.0", "event": event, "data": [data]})
    );
}

fn fmt_near(yocto: u128) -> String {
    // two decimals, rounded down
    let hundredths = yocto / 10_000_000_000_000_000_000_000;
    format!("{}.{:02}", hundredths / 100, hundredths % 100)
}

fn xml_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '&' => "&amp;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

#[near]
impl Contract {
    #[init]
    pub fn new(owner_id: AccountId) -> Self {
        Self {
            tokens: NonFungibleToken::new(
                StorageKey::NonFungibleToken,
                owner_id,
                Some(StorageKey::TokenMetadata),
                Some(StorageKey::Enumeration),
                None::<StorageKey>,
            ),
            metadata: LazyOption::new(
                StorageKey::Metadata,
                Some(&NFTContractMetadata {
                    spec: NFT_METADATA_SPEC.to_string(),
                    name: "PocketCards".to_string(),
                    symbol: "POCKET".to_string(),
                    icon: None,
                    base_uri: None,
                    reference: None,
                    reference_hash: None,
                }),
            ),
            cards: LookupMap::new(StorageKey::Cards),
            next_id: 1,
        }
    }

    /// Mint a funded card to `receiver_id`. The attached deposit pays for storage;
    /// everything above that becomes the card's balance. Returns the new token id.
    #[payable]
    pub fn mint_card(
        &mut self,
        receiver_id: AccountId,
        title: String,
        daily_limit: U128,
        allowlist: Option<Vec<AccountId>>,
        expires_at_ms: Option<U64>,
    ) -> TokenId {
        let now = env::block_timestamp();
        let title = title.trim().to_string();
        require!(
            !title.is_empty() && title.chars().count() <= MAX_TITLE_LEN,
            "title must be 1-48 characters"
        );
        let allowlist = allowlist.filter(|a| !a.is_empty());
        if let Some(a) = &allowlist {
            require!(
                a.len() <= MAX_ALLOWLIST,
                "allowlist can hold at most 10 accounts"
            );
        }
        let expires_at_ns = expires_at_ms.map(|ms| ms.0.saturating_mul(1_000_000));
        if let Some(e) = expires_at_ns {
            require!(e > now, "expires_at_ms must be in the future");
        }

        let storage_before = env::storage_usage();
        let token_id = self.next_id.to_string();
        self.next_id += 1;
        let issuer = env::predecessor_account_id();

        self.tokens.internal_mint_with_refund(
            token_id.clone(),
            receiver_id.clone(),
            Some(TokenMetadata {
                title: Some(title.clone()),
                description: None,
                media: None,
                media_hash: None,
                copies: Some(1),
                issued_at: Some((now / 1_000_000).to_string()),
                expires_at: expires_at_ms.map(|m| m.0.to_string()),
                starts_at: None,
                updated_at: None,
                extra: None,
                reference: None,
                reference_hash: None,
            }),
            None,
        );
        self.cards.insert(
            token_id.clone(),
            Card {
                issuer: issuer.clone(),
                title,
                balance: 0,
                daily_limit: daily_limit.0,
                allowlist,
                expires_at_ns,
                created_at_ns: now,
                window_start_ns: now,
                spent_in_window: 0,
                total_funded: 0,
                total_spent: 0,
                spend_count: 0,
                last_spend: None,
                reclaimed: false,
            },
        );
        self.cards.flush();

        let bytes = env::storage_usage() - storage_before + STORAGE_RESERVE_BYTES;
        let storage_cost = env::storage_byte_cost().saturating_mul(bytes as u128);
        let deposit = env::attached_deposit();
        require!(
            deposit >= storage_cost,
            format!(
                "attach at least {} yoctoNEAR to cover storage",
                storage_cost.as_yoctonear()
            )
        );
        let funded = deposit.saturating_sub(storage_cost).as_yoctonear();
        let card = self.cards.get_mut(&token_id).unwrap();
        card.balance = funded;
        card.total_funded = funded;

        near_contract_standards::non_fungible_token::events::NftMint {
            owner_id: &receiver_id,
            token_ids: &[&token_id],
            memo: None,
        }
        .emit();
        emit(
            "mint",
            json!({"token_id": token_id, "issuer": issuer, "owner_id": receiver_id,
                   "funded": funded.to_string(), "storage_cost": storage_cost.as_yoctonear().to_string()}),
        );
        token_id
    }

    /// Anyone can add NEAR to any card (parents, employers, other agents...).
    #[payable]
    pub fn top_up(&mut self, token_id: TokenId) -> U128 {
        let amount = env::attached_deposit().as_yoctonear();
        require!(amount > 0, "attach NEAR to top up");
        let card = self
            .cards
            .get_mut(&token_id)
            .unwrap_or_else(|| env::panic_str("no such card"));
        require!(!card.reclaimed, "card was reclaimed by its issuer");
        require!(!card.is_expired(env::block_timestamp()), "card has expired");
        card.balance += amount;
        card.total_funded += amount;
        let balance = card.balance;
        emit(
            "top_up",
            json!({"token_id": token_id, "from": env::predecessor_account_id(), "amount": amount.to_string()}),
        );
        U128(balance)
    }

    /// The current NFT owner pays `amount` yoctoNEAR from the card to `receiver_id`,
    /// subject to the card's expiry, allowlist and rolling 24h limit.
    pub fn spend(&mut self, token_id: TokenId, receiver_id: AccountId, amount: U128) -> Promise {
        let now = env::block_timestamp();
        let caller = env::predecessor_account_id();
        let owner = self
            .tokens
            .owner_by_id
            .get(&token_id)
            .unwrap_or_else(|| env::panic_str("no such card"));
        require!(caller == owner, "only the card holder can spend");
        let card = self.cards.get_mut(&token_id).unwrap();
        require!(!card.reclaimed, "card was reclaimed by its issuer");
        require!(!card.is_expired(now), "card has expired");
        require!(amount.0 > 0, "amount must be positive");
        if let Some(list) = &card.allowlist {
            require!(
                list.contains(&receiver_id),
                "receiver is not on this card's allowlist"
            );
        }
        require!(amount.0 <= card.balance, "insufficient card balance");
        let (spent, start) = card.window(now);
        if card.daily_limit > 0 {
            require!(
                spent + amount.0 <= card.daily_limit,
                "daily limit exceeded for this card"
            );
        }
        card.window_start_ns = start;
        card.spent_in_window = spent + amount.0;
        card.balance -= amount.0;
        card.total_spent += amount.0;
        card.spend_count += 1;
        card.last_spend = Some(LastSpend {
            to: receiver_id.clone(),
            amount,
            at_ms: U64(now / 1_000_000),
        });
        emit(
            "spend",
            json!({"token_id": token_id, "by": caller, "to": receiver_id, "amount": amount}),
        );
        Promise::new(receiver_id)
            .transfer(NearToken::from_yoctonear(amount.0))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_FOR_CALLBACK)
                    .on_payout(token_id, amount, false),
            )
    }

    /// After expiry, the issuer takes back whatever is left on the card.
    pub fn reclaim(&mut self, token_id: TokenId) -> Promise {
        let now = env::block_timestamp();
        let card = self
            .cards
            .get_mut(&token_id)
            .unwrap_or_else(|| env::panic_str("no such card"));
        require!(
            env::predecessor_account_id() == card.issuer,
            "only the issuer can reclaim"
        );
        require!(card.is_expired(now), "card has not expired yet");
        require!(!card.reclaimed, "already reclaimed");
        require!(card.balance > 0, "nothing to reclaim");
        let amount = card.balance;
        card.balance = 0;
        card.reclaimed = true;
        let issuer = card.issuer.clone();
        emit(
            "reclaim",
            json!({"token_id": token_id, "issuer": issuer, "amount": amount.to_string()}),
        );
        Promise::new(issuer)
            .transfer(NearToken::from_yoctonear(amount))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_FOR_CALLBACK)
                    .on_payout(token_id, U128(amount), true),
            )
    }

    /// Restores the card if a payout transfer failed (e.g. receiver account does not exist).
    #[private]
    pub fn on_payout(
        &mut self,
        token_id: TokenId,
        amount: U128,
        is_reclaim: bool,
        #[callback_result] result: Result<(), PromiseError>,
    ) -> bool {
        if result.is_ok() {
            return true;
        }
        let card = self.cards.get_mut(&token_id).unwrap();
        card.balance += amount.0;
        if is_reclaim {
            card.reclaimed = false;
        } else {
            card.spent_in_window = card.spent_in_window.saturating_sub(amount.0);
            card.total_spent = card.total_spent.saturating_sub(amount.0);
            card.spend_count = card.spend_count.saturating_sub(1);
            card.last_spend = None;
        }
        emit(
            "payout_failed",
            json!({"token_id": token_id, "amount": amount, "restored": true}),
        );
        false
    }

    // ---------- views ----------

    pub fn get_card(&self, token_id: TokenId) -> Option<CardView> {
        let owner = self.tokens.owner_by_id.get(&token_id)?;
        let card = self.cards.get(&token_id)?;
        Some(self.view(token_id, owner, card))
    }

    pub fn cards_for_owner(&self, account_id: AccountId) -> Vec<CardView> {
        self.tokens
            .nft_tokens_for_owner(account_id, None, Some(50))
            .into_iter()
            .filter_map(|t| self.get_card(t.token_id))
            .collect()
    }

    /// Latest cards first, handy for a public gallery.
    pub fn recent_cards(&self, limit: Option<u64>) -> Vec<CardView> {
        let limit = limit.unwrap_or(10).min(50);
        (1..self.next_id)
            .rev()
            .take(limit as usize)
            .filter_map(|i| self.get_card(i.to_string()))
            .collect()
    }

    /// Live SVG for a card (also embedded in `nft_token().metadata.media`).
    pub fn card_svg(&self, token_id: TokenId) -> Option<String> {
        self.get_card(token_id).map(|v| render_svg(&v))
    }
}

impl Contract {
    fn view(&self, token_id: TokenId, owner_id: AccountId, card: &Card) -> CardView {
        let now = env::block_timestamp();
        let (spent, start) = card.window(now);
        CardView {
            token_id,
            owner_id,
            issuer: card.issuer.clone(),
            title: card.title.clone(),
            balance: U128(card.balance),
            daily_limit: U128(card.daily_limit),
            spent_today: U128(spent),
            available_now: U128(card.available_now(now)),
            window_resets_at_ms: U64((start + DAY_NS) / 1_000_000),
            allowlist: card.allowlist.clone(),
            expires_at_ms: card.expires_at_ns.map(|e| U64(e / 1_000_000)),
            is_expired: card.is_expired(now) || card.reclaimed,
            created_at_ms: U64(card.created_at_ns / 1_000_000),
            total_funded: U128(card.total_funded),
            total_spent: U128(card.total_spent),
            spend_count: U64(card.spend_count),
            last_spend: card.last_spend.clone(),
        }
    }

    /// Adds live media + state to the stored NEP-177 metadata.
    fn decorate(&self, mut token: Token) -> Token {
        if let Some(v) = self.get_card(token.token_id.clone()) {
            let svg = render_svg(&v);
            let b64 = near_sdk::base64::engine::general_purpose::STANDARD.encode(svg);
            let mut md = token.metadata.unwrap_or_default();
            md.media = Some(format!("data:image/svg+xml;base64,{b64}"));
            md.description = Some(format!(
                "Holds {} NEAR. Spend cap {} per 24h.{} Issued by {}.",
                fmt_near(v.balance.0),
                if v.daily_limit.0 == 0 {
                    "none".to_string()
                } else {
                    format!("{} NEAR", fmt_near(v.daily_limit.0))
                },
                if v.allowlist.is_some() {
                    " Pays allowlisted accounts only."
                } else {
                    ""
                },
                v.issuer
            ));
            md.extra = near_sdk::serde_json::to_string(&v).ok();
            token.metadata = Some(md);
        }
        token
    }
}

pub fn render_svg(v: &CardView) -> String {
    let (c1, c2) = if v.is_expired {
        ("#6b7280", "#374151")
    } else {
        ("#00ec97", "#0b6e4f")
    };
    let limit = if v.daily_limit.0 == 0 {
        "no daily cap".to_string()
    } else {
        format!(
            "today {} / {} NEAR",
            fmt_near(v.spent_today.0),
            fmt_near(v.daily_limit.0)
        )
    };
    let status = if v.is_expired {
        "EXPIRED".to_string()
    } else if v.allowlist.is_some() {
        format!(
            "{} allowlisted payee(s)",
            v.allowlist.as_ref().map(|a| a.len()).unwrap_or(0)
        )
    } else {
        "pays anyone".to_string()
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 250" width="400" height="250"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{c1}"/><stop offset="1" stop-color="{c2}"/></linearGradient></defs><rect width="400" height="250" rx="22" fill="url(#g)"/><text x="24" y="40" font-family="monospace" font-size="15" fill="#06231a">POCKETCARD #{id}</text><text x="24" y="70" font-family="sans-serif" font-size="20" font-weight="bold" fill="#fff">{title}</text><text x="24" y="135" font-family="sans-serif" font-size="40" font-weight="bold" fill="#fff">{bal} NEAR</text><text x="24" y="170" font-family="monospace" font-size="14" fill="#eafff6">{limit}</text><text x="24" y="195" font-family="monospace" font-size="14" fill="#eafff6">{status}</text><text x="24" y="228" font-family="monospace" font-size="11" fill="#06231a">holder {owner}</text></svg>"##,
        id = xml_escape(&v.token_id),
        title = xml_escape(&v.title),
        bal = fmt_near(v.balance.0),
        owner = xml_escape(v.owner_id.as_str()),
    )
}

// ---------- NEP-171 core / resolver / enumeration / metadata ----------

#[near]
impl NonFungibleTokenCore for Contract {
    #[payable]
    fn nft_transfer(
        &mut self,
        receiver_id: AccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        memo: Option<String>,
    ) {
        self.tokens
            .nft_transfer(receiver_id, token_id, approval_id, memo)
    }

    #[payable]
    fn nft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<bool> {
        self.tokens
            .nft_transfer_call(receiver_id, token_id, approval_id, memo, msg)
    }

    fn nft_token(&self, token_id: TokenId) -> Option<Token> {
        self.tokens.nft_token(token_id).map(|t| self.decorate(t))
    }
}

#[near]
impl NonFungibleTokenResolver for Contract {
    #[private]
    fn nft_resolve_transfer(
        &mut self,
        previous_owner_id: AccountId,
        receiver_id: AccountId,
        token_id: TokenId,
        approved_account_ids: Option<HashMap<AccountId, u64>>,
    ) -> bool {
        self.tokens.nft_resolve_transfer(
            previous_owner_id,
            receiver_id,
            token_id,
            approved_account_ids,
        )
    }
}

#[near]
impl NonFungibleTokenEnumeration for Contract {
    fn nft_total_supply(&self) -> U128 {
        self.tokens.nft_total_supply()
    }

    fn nft_tokens(&self, from_index: Option<U128>, limit: Option<u64>) -> Vec<Token> {
        self.tokens
            .nft_tokens(from_index, limit)
            .into_iter()
            .map(|t| self.decorate(t))
            .collect()
    }

    fn nft_supply_for_owner(&self, account_id: AccountId) -> U128 {
        self.tokens.nft_supply_for_owner(account_id)
    }

    fn nft_tokens_for_owner(
        &self,
        account_id: AccountId,
        from_index: Option<U128>,
        limit: Option<u64>,
    ) -> Vec<Token> {
        self.tokens
            .nft_tokens_for_owner(account_id, from_index, limit)
            .into_iter()
            .map(|t| self.decorate(t))
            .collect()
    }
}

#[near]
impl NonFungibleTokenMetadataProvider for Contract {
    fn nft_metadata(&self) -> NFTContractMetadata {
        self.metadata.get().unwrap()
    }
}

#[cfg(test)]
mod tests;
