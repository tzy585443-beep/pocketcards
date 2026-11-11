# PocketCards: NFTs that hold money and spend it by rules

**Track:** NFTs That Do Something (`nfts`) · **Network:** NEAR testnet · **Built for:** NEAR HACKATHON 2026 (Nov 11–14, 2026)

- **Live contract:** [`pocketcards.testnet`](https://testnet.nearblocks.io/address/pocketcards.testnet)
- **Hosted app:** https://tzy585443-beep.github.io/pocketcards/
- **Demo video (2:21, screen recording with captions and voiceover):** https://tzy585443-beep.github.io/pocketcards/demo/pocketcards-demo.mp4

## The problem

Giving someone money on-chain today means giving them all of it. A parent can't send pocket money that only works at the school cafe, a company can't issue a travel stipend that tops out at $50 a day, and a gift card can't expire and go back to the sender. You can build these as separate custodial apps, but then the rules live in a database and nothing moves between wallets.

## What PocketCards is

A PocketCard is a **standard NEP-171 NFT that carries its own NEAR balance plus a spending policy** set by whoever issued it:

| Rule | What the contract enforces |
| --- | --- |
| `daily_limit` | The holder can spend at most this much per rolling 24 h window (0 = no cap). |
| `allowlist` | Optional list of up to 10 accounts the card may pay (merchants, an API provider, a cafe). |
| `expires_at_ms` | Optional expiry. After it the card can't spend, and the **issuer** can `reclaim` what's left. |

Whoever holds the NFT can `spend` from it within those rules. Transferring the NFT (any NEP-171 wallet or `nft_transfer`) hands over **the money and the rules together**. Anyone can `top_up` a card. The token's `metadata.media` is an **on-chain SVG generated on every view**, so wallets and marketplaces always show the live balance and today's remaining allowance. `metadata.extra` carries the full card state as JSON.

Use cases: kids' allowances, employee or contractor stipends, expiring gift cards, event credits, and **capped budgets for AI agents** (send an agent a card instead of a key to your wallet; it can only pay allowlisted APIs up to a daily limit).

### How NEAR fits

- **NEP-171 / NEP-177 / NEP-181** (core, metadata, enumeration) via `near-contract-standards`, so the card shows up in NEAR wallets and can be moved with a standard transfer.
- **Named accounts** make allowlists readable (`cafeteria.testnet`, not a hex address).
- **Async receipts with callbacks:** a payout is a `Transfer` receipt followed by an `on_payout` callback. If the transfer fails (for example the receiver doesn't exist), the callback **restores the card balance and the daily allowance**. This is covered by the sandbox test.
- **Storage staking:** the minter's deposit pays for the NFT's storage (≈0.01 NEAR, measured on-chain); everything above that becomes the card balance.

## Try it (judges): do X, see Y

### In the browser (no install)

1. Open https://tzy585443-beep.github.io/pocketcards/. **You should see** "Recent cards" with live card art from `pocketcards.testnet`, for example card #4 owned by `pc-zoe.testnet` with `today 0.15 / 0.20 NEAR`.
2. Type `pc-zoe.testnet` in "Look up an account". **You should see** its card: balance ≈0.84 NEAR, payable only to `pc-cafe.testnet`, one payment made.
3. Click **Connect wallet** (MyNearWallet, testnet) or expand **"use a testnet dev key"** and paste a testnet account and key created with `near-cli`.
4. In **Issue a card**, give yourself a card: load 0.5 NEAR, daily cap 0.1, leave the allowlist empty. **You should see** a new card under **My cards** with about 0.49 NEAR and an "Activity" row with a tx link.
5. On that card, **Spend** 0.06 NEAR to any testnet account. **You should see** "done"; the balance drops and "Spendable now" shows 0.04.
6. Spend 0.06 again. **You should see** the contract reject it: `daily limit exceeded for this card`.
7. **Give card** to another account. **You should see** it disappear from My cards; it now appears under that account in Look up, with the same balance and the same remaining daily allowance (moving the card doesn't reset its window).

### From the command line (near-cli-rs)

```bash
# read-only, no account needed
near contract call-function as-read-only pocketcards.testnet recent_cards json-args '{"limit":3}' network-config testnet now
near contract call-function as-read-only pocketcards.testnet nft_token json-args '{"token_id":"4"}' network-config testnet now   # metadata.media = live SVG

# with a testnet account (create one free: near account create-account sponsor-by-faucet-service <name>.testnet autogenerate-new-keypair save-to-legacy-keychain network-config testnet create)
near contract call-function as-transaction pocketcards.testnet mint_card \
  json-args '{"receiver_id":"<you>.testnet","title":"Test card","daily_limit":"100000000000000000000000","allowlist":null,"expires_at_ms":null}' \
  prepaid-gas '50 Tgas' attached-deposit '0.5 NEAR' sign-as <you>.testnet network-config testnet sign-with-legacy-keychain send
near contract call-function as-transaction pocketcards.testnet spend \
  json-args '{"token_id":"<id>","receiver_id":"pc-cafe.testnet","amount":"60000000000000000000000"}' \
  prepaid-gas '50 Tgas' attached-deposit '0 NEAR' sign-as <you>.testnet network-config testnet sign-with-legacy-keychain send
```

## Contract API

| Method | Who | Notes |
| --- | --- | --- |
| `new(owner_id)` | deployer | init |
| `mint_card(receiver_id, title, daily_limit, allowlist?, expires_at_ms?)` payable | anyone | deposit − storage = card balance; returns `token_id` |
| `top_up(token_id)` payable | anyone | blocked after expiry/reclaim |
| `spend(token_id, receiver_id, amount)` | current NFT owner | checks expiry, allowlist, 24 h cap, balance; transfer + `on_payout` callback |
| `reclaim(token_id)` | issuer, after expiry | sends remaining balance to issuer |
| `nft_transfer`, `nft_transfer_call`, `nft_token`, `nft_tokens`, `nft_tokens_for_owner`, `nft_supply_for_owner`, `nft_total_supply`, `nft_metadata` | standard | `nft_token` adds the live SVG, description and `extra` |
| `get_card(token_id)`, `cards_for_owner(account_id)`, `recent_cards(limit?)`, `card_svg(token_id)` | view | JSON card state |

Amounts are yoctoNEAR strings (`U128`). Events are emitted as NEP-297 `EVENT_JSON` logs (`nft_mint` plus `pocketcards` `mint`/`top_up`/`spend`/`reclaim`/`payout_failed`).

## Repository layout

```
src/lib.rs          contract (Rust, near-sdk 5.30, near-contract-standards)
src/tests.rs        22 unit tests (limits, window reset, allowlist, expiry/reclaim, transfer, failed-payout restore, SVG escaping)
tests/sandbox.rs    end-to-end test on a local NEAR sandbox node: real transfers, real callbacks
web/                frontend source (vanilla JS + near-api-js + wallet-selector), built with esbuild into docs/
docs/               GitHub Pages: hosted app + demo video
scripts/            how the demo video was produced (puppeteer screen recording + captions + TTS)
```

## Build and test

Requirements: Rust (the toolchain in `rust-toolchain.toml` is installed automatically by rustup), [`cargo-near`](https://github.com/near/cargo-near), Node 20+ for the frontend.

```bash
cargo test                      # 22 unit tests + sandbox end-to-end test (downloads a sandbox node on first run)
cargo near build non-reproducible-wasm
near contract deploy <account>.testnet use-file target/near/pocketcards.wasm \
  with-init-call new json-args '{"owner_id":"<account>.testnet"}' prepaid-gas '30 Tgas' attached-deposit '0 NEAR' \
  network-config testnet sign-with-legacy-keychain send

cd web && npm install && node build.mjs   # writes docs/; serve with: python3 -m http.server -d ../docs
# point the app at another deployment: https://.../?contract=<account>.testnet
```

On some Linux hosts the sandbox test's OpenSSL build needs `OPENSSL_NO_VENDOR=1` (with system libssl headers installed).

## Security notes and limits

- Only the current NFT owner can spend; the issuer can't spend a card it gave away, and can only take funds back after the expiry it chose at mint.
- State is updated before the transfer and rolled back in `on_payout` if the transfer fails, so a failed payout can't drain or double-count a card.
- The 24 h window travels with the card, so moving it between your own accounts doesn't reset the cap.
- Card rules are fixed at mint. No admin keys, no upgrade method, no approvals extension (no marketplace escrow) in this version.
- Testnet only. It hasn't been audited; don't put mainnet funds in it. NEAR-native balances only; NEP-141 stablecoin cards are the obvious next step.
- The "testnet dev key" option in the web app keeps a pasted testnet key in `sessionStorage` for that tab only; it is meant for judges using `near-cli` accounts. Use a wallet for anything real.

## Disclosure (hackathon rules Q-001–Q-005)

- Everything in this repository was written during the hackathon window (first commit Nov 11, 2026). No earlier code.
- Built by a solo team with an AI coding agent. Libraries and templates used: `cargo near new` project template, `near-sdk` 5.30, `near-contract-standards` 5.30, `near-sandbox`, `near-api` (tests), `near-api-js` 7, `@near-wallet-selector` 10 (core, modal-ui, my-near-wallet), esbuild, puppeteer-core and edge-tts (demo video only).
- This team has no other NEAR HACKATHON submission.

## License

MIT, see [LICENSE](LICENSE).
