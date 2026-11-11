use super::*;
use near_sdk::test_utils::{VMContextBuilder, get_logs};
use near_sdk::testing_env;

const NEAR: u128 = 1_000_000_000_000_000_000_000_000;
const T0: u64 = 1_800_000_000_000_000_000; // ns

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}

fn ctx(pred: &str, deposit: u128, ts: u64) {
    let mut b = VMContextBuilder::new();
    b.current_account_id(acc("cards.testnet"))
        .predecessor_account_id(acc(pred))
        .signer_account_id(acc(pred))
        .attached_deposit(NearToken::from_yoctonear(deposit))
        .block_timestamp(ts);
    testing_env!(b.build());
}

fn setup() -> Contract {
    ctx("cards.testnet", 0, T0);
    Contract::new(acc("cards.testnet"))
}

/// mom mints a 5 NEAR card for kid with a 1 NEAR/day cap.
fn mint_default(
    c: &mut Contract,
    allow: Option<Vec<AccountId>>,
    expires_ms: Option<u64>,
) -> TokenId {
    ctx("mom.testnet", 5 * NEAR, T0);
    c.mint_card(
        acc("kid.testnet"),
        "Pocket money".into(),
        U128(NEAR),
        allow,
        expires_ms.map(U64),
    )
}

#[test]
fn mint_funds_card_minus_storage() {
    let mut c = setup();
    let id = mint_default(&mut c, None, None);
    assert_eq!(id, "1");
    let v = c.get_card(id.clone()).unwrap();
    assert_eq!(v.owner_id, acc("kid.testnet"));
    assert_eq!(v.issuer, acc("mom.testnet"));
    // storage is a small fraction of the deposit
    assert!(
        v.balance.0 < 5 * NEAR && v.balance.0 > 49 * NEAR / 10,
        "{}",
        v.balance.0
    );
    assert_eq!(v.total_funded, v.balance);
    assert_eq!(v.available_now.0, NEAR);
    assert!(
        get_logs()
            .iter()
            .any(|l| l.contains("\"event\":\"nft_mint\""))
    );
    assert!(get_logs().iter().any(|l| l.contains("\"event\":\"mint\"")));
}

#[test]
#[should_panic(expected = "cover storage")]
fn mint_requires_storage_deposit() {
    let mut c = setup();
    ctx("mom.testnet", 1, T0);
    c.mint_card(acc("kid.testnet"), "x".into(), U128(0), None, None);
}

#[test]
#[should_panic(expected = "title must be 1-48 characters")]
fn mint_rejects_empty_title() {
    let mut c = setup();
    ctx("mom.testnet", NEAR, T0);
    c.mint_card(acc("kid.testnet"), "   ".into(), U128(0), None, None);
}

#[test]
#[should_panic(expected = "expires_at_ms must be in the future")]
fn mint_rejects_past_expiry() {
    let mut c = setup();
    ctx("mom.testnet", NEAR, T0);
    c.mint_card(
        acc("kid.testnet"),
        "x".into(),
        U128(0),
        None,
        Some(U64(T0 / 1_000_000 - 1)),
    );
}

#[test]
fn holder_spends_within_limit() {
    let mut c = setup();
    let id = mint_default(&mut c, None, None);
    let before = c.get_card(id.clone()).unwrap().balance.0;
    ctx("kid.testnet", 0, T0 + 1);
    let _ = c.spend(id.clone(), acc("shop.testnet"), U128(NEAR * 6 / 10));
    let v = c.get_card(id.clone()).unwrap();
    assert_eq!(v.balance.0, before - NEAR * 6 / 10);
    assert_eq!(v.spent_today.0, NEAR * 6 / 10);
    assert_eq!(v.available_now.0, NEAR * 4 / 10);
    assert_eq!(v.spend_count.0, 1);
    assert_eq!(v.last_spend.unwrap().to, acc("shop.testnet"));
}

#[test]
#[should_panic(expected = "daily limit exceeded")]
fn daily_limit_enforced() {
    let mut c = setup();
    let id = mint_default(&mut c, None, None);
    ctx("kid.testnet", 0, T0 + 1);
    let _ = c.spend(id.clone(), acc("shop.testnet"), U128(NEAR * 6 / 10));
    let _ = c.spend(id, acc("shop.testnet"), U128(NEAR * 6 / 10));
}

#[test]
fn limit_resets_after_24h() {
    let mut c = setup();
    let id = mint_default(&mut c, None, None);
    ctx("kid.testnet", 0, T0 + 1);
    let _ = c.spend(id.clone(), acc("shop.testnet"), U128(NEAR));
    assert_eq!(c.get_card(id.clone()).unwrap().available_now.0, 0);
    ctx("kid.testnet", 0, T0 + 1 + DAY_NS);
    assert_eq!(c.get_card(id.clone()).unwrap().available_now.0, NEAR);
    let _ = c.spend(id.clone(), acc("shop.testnet"), U128(NEAR));
    assert_eq!(c.get_card(id).unwrap().total_spent.0, 2 * NEAR);
}

#[test]
#[should_panic(expected = "only the card holder can spend")]
fn issuer_cannot_spend_holders_card() {
    let mut c = setup();
    let id = mint_default(&mut c, None, None);
    ctx("mom.testnet", 0, T0 + 1);
    let _ = c.spend(id, acc("mom.testnet"), U128(1));
}

#[test]
#[should_panic(expected = "not on this card's allowlist")]
fn allowlist_blocks_other_receivers() {
    let mut c = setup();
    let id = mint_default(&mut c, Some(vec![acc("cafeteria.testnet")]), None);
    ctx("kid.testnet", 0, T0 + 1);
    let _ = c.spend(id, acc("random.testnet"), U128(1));
}

#[test]
fn allowlist_allows_listed_receiver() {
    let mut c = setup();
    let id = mint_default(&mut c, Some(vec![acc("cafeteria.testnet")]), None);
    ctx("kid.testnet", 0, T0 + 1);
    let _ = c.spend(id.clone(), acc("cafeteria.testnet"), U128(1000));
    assert_eq!(c.get_card(id).unwrap().spend_count.0, 1);
}

#[test]
#[should_panic(expected = "insufficient card balance")]
fn cannot_overspend_balance() {
    let mut c = setup();
    ctx("mom.testnet", NEAR / 10, T0);
    let id = c.mint_card(acc("kid.testnet"), "tiny".into(), U128(0), None, None);
    ctx("kid.testnet", 0, T0 + 1);
    let _ = c.spend(id, acc("shop.testnet"), U128(NEAR));
}

#[test]
fn transfer_moves_money_and_rules_and_keeps_window() {
    let mut c = setup();
    let id = mint_default(&mut c, None, None);
    ctx("kid.testnet", 0, T0 + 1);
    let _ = c.spend(id.clone(), acc("shop.testnet"), U128(NEAR * 9 / 10));
    ctx("kid.testnet", 1, T0 + 2);
    c.nft_transfer(acc("friend.testnet"), id.clone(), None, None);
    let v = c.get_card(id.clone()).unwrap();
    assert_eq!(v.owner_id, acc("friend.testnet"));
    // window carries over: passing the card to a second wallet does not reset the cap
    assert_eq!(v.available_now.0, NEAR / 10);
    assert!(c.cards_for_owner(acc("kid.testnet")).is_empty());
    assert_eq!(c.cards_for_owner(acc("friend.testnet")).len(), 1);
}

#[test]
#[should_panic(expected = "only the card holder can spend")]
fn old_holder_loses_spending_after_transfer() {
    let mut c = setup();
    let id = mint_default(&mut c, None, None);
    ctx("kid.testnet", 1, T0 + 1);
    c.nft_transfer(acc("friend.testnet"), id.clone(), None, None);
    ctx("kid.testnet", 0, T0 + 2);
    let _ = c.spend(id, acc("shop.testnet"), U128(1));
}

#[test]
fn anyone_can_top_up() {
    let mut c = setup();
    let id = mint_default(&mut c, None, None);
    let b = c.get_card(id.clone()).unwrap().balance.0;
    ctx("grandpa.testnet", 2 * NEAR, T0 + 5);
    let nb = c.top_up(id.clone());
    assert_eq!(nb.0, b + 2 * NEAR);
    assert_eq!(c.get_card(id).unwrap().total_funded.0, b + 2 * NEAR);
}

#[test]
#[should_panic(expected = "card has expired")]
fn expired_card_cannot_spend() {
    let mut c = setup();
    let exp = T0 / 1_000_000 + 60_000;
    let id = mint_default(&mut c, None, Some(exp));
    ctx("kid.testnet", 0, T0 + 61 * 1_000_000_000);
    let _ = c.spend(id, acc("shop.testnet"), U128(1));
}

#[test]
fn issuer_reclaims_after_expiry() {
    let mut c = setup();
    let exp = T0 / 1_000_000 + 60_000;
    let id = mint_default(&mut c, None, Some(exp));
    ctx("mom.testnet", 0, T0 + 61 * 1_000_000_000);
    let _ = c.reclaim(id.clone());
    let v = c.get_card(id).unwrap();
    assert_eq!(v.balance.0, 0);
    assert!(v.is_expired);
    assert_eq!(v.available_now.0, 0);
}

#[test]
#[should_panic(expected = "card has not expired yet")]
fn issuer_cannot_reclaim_early() {
    let mut c = setup();
    let exp = T0 / 1_000_000 + 60_000;
    let id = mint_default(&mut c, None, Some(exp));
    ctx("mom.testnet", 0, T0 + 1);
    let _ = c.reclaim(id);
}

#[test]
#[should_panic(expected = "only the issuer can reclaim")]
fn stranger_cannot_reclaim() {
    let mut c = setup();
    let exp = T0 / 1_000_000 + 60_000;
    let id = mint_default(&mut c, None, Some(exp));
    ctx("kid.testnet", 0, T0 + 61 * 1_000_000_000);
    let _ = c.reclaim(id);
}

#[test]
fn failed_payout_restores_card() {
    let mut c = setup();
    let id = mint_default(&mut c, None, None);
    let b = c.get_card(id.clone()).unwrap().balance.0;
    ctx("kid.testnet", 0, T0 + 1);
    let _ = c.spend(id.clone(), acc("ghost.testnet"), U128(NEAR / 2));
    ctx("cards.testnet", 0, T0 + 2);
    let ok = c.on_payout(id.clone(), U128(NEAR / 2), false, Err(PromiseError::Failed));
    assert!(!ok);
    let v = c.get_card(id).unwrap();
    assert_eq!(v.balance.0, b);
    assert_eq!(v.spent_today.0, 0);
    assert_eq!(v.spend_count.0, 0);
}

#[test]
fn nft_token_has_live_svg_media() {
    let mut c = setup();
    let id = mint_default(&mut c, None, None);
    let t = c.nft_token(id.clone()).unwrap();
    let md = t.metadata.unwrap();
    assert!(md.media.unwrap().starts_with("data:image/svg+xml;base64,"));
    assert!(md.extra.unwrap().contains("\"daily_limit\""));
    let svg = c.card_svg(id).unwrap();
    assert!(svg.contains("Pocket money") && svg.contains(" NEAR</text>"));
    assert_eq!(c.nft_tokens(None, None).len(), 1);
    assert_eq!(c.recent_cards(None).len(), 1);
}

#[test]
fn svg_escapes_title() {
    let mut c = setup();
    ctx("mom.testnet", NEAR, T0);
    let id = c.mint_card(
        acc("kid.testnet"),
        "<b>&\"x\"</b>".into(),
        U128(0),
        None,
        None,
    );
    let svg = c.card_svg(id).unwrap();
    assert!(!svg.contains("<b>"));
    assert!(svg.contains("&lt;b&gt;&amp;&quot;x&quot;"));
}

#[test]
fn fmt_near_rounds_down() {
    assert_eq!(fmt_near(NEAR * 3 / 2), "1.50");
    assert_eq!(fmt_near(NEAR / 1000), "0.00");
    assert_eq!(fmt_near(12 * NEAR + NEAR / 100 * 7), "12.07");
}
