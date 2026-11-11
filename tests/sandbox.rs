//! End-to-end test on a local NEAR sandbox node: real runtime, real transfers, real callbacks.
use near_api::{AccountId, NearGas, NearToken};
use near_sdk::serde_json::{Value, json};

const TGAS30: NearGas = NearGas::from_tgas(30);

async fn create(
    sandbox: &near_sandbox::Sandbox,
    name: &str,
    near: u128,
) -> testresult::TestResult<near_api::Account> {
    let id: AccountId = name.parse()?;
    sandbox
        .create_account(id.as_str().parse()?)
        .initial_balance(NearToken::from_near(near))
        .send()
        .await?;
    Ok(near_api::Account(id))
}

#[tokio::test]
async fn pocketcards_end_to_end() -> testresult::TestResult<()> {
    let wasm = std::fs::read(cargo_near_build::build_with_cli(Default::default())?)?;
    let sandbox = near_sandbox::Sandbox::start_sandbox().await?;
    let net = near_api::NetworkConfig::from_rpc_url("sandbox", sandbox.rpc_addr.parse()?);
    let signer = near_api::Signer::from_secret_key(
        near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse()?,
    )?;

    let contract = create(&sandbox, "cards.sandbox", 20).await?.as_contract();
    let mom = create(&sandbox, "mom.sandbox", 20).await?;
    let kid = create(&sandbox, "kid.sandbox", 5).await?;
    let shop = create(&sandbox, "shop.sandbox", 1).await?;
    let friend = create(&sandbox, "friend.sandbox", 5).await?;

    near_api::Contract::deploy(contract.account_id().clone())
        .use_code(wasm)
        .with_init_call("new", json!({"owner_id": contract.account_id()}))?
        .with_signer(signer.clone())
        .send_to(&net)
        .await?
        .assert_success();

    // mom mints a 3 NEAR card for kid: 1 NEAR/day, may only pay shop.sandbox
    let mint = contract
        .call_function(
            "mint_card",
            json!({"receiver_id": kid.account_id(), "title": "Lunch money", "daily_limit": NearToken::from_near(1).as_yoctonear().to_string(),
                   "allowlist": [shop.account_id()], "expires_at_ms": null}),
        )
        .transaction()
        .deposit(NearToken::from_near(3))
        .gas(TGAS30)
        .with_signer(mom.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?;
    mint.assert_success();

    let card = |net: near_api::NetworkConfig| {
        let c = contract.clone();
        async move {
            let v: Value = c
                .call_function("get_card", json!({"token_id": "1"}))
                .read_only()
                .fetch_from(&net)
                .await
                .unwrap()
                .data;
            v
        }
    };
    let v = card(net.clone()).await;
    assert_eq!(v["owner_id"], kid.account_id().to_string());
    let bal0: u128 = v["balance"].as_str().unwrap().parse()?;
    println!("card #1 balance after storage: {bal0}");
    assert!(bal0 > NearToken::from_millinear(2950).as_yoctonear());

    // kid pays the shop 0.4 NEAR -> shop balance goes up
    let shop_before = shop.tokens().near_balance().fetch_from(&net).await?.total;
    contract
        .call_function("spend", json!({"token_id": "1", "receiver_id": shop.account_id(), "amount": NearToken::from_millinear(400).as_yoctonear().to_string()}))
        .transaction()
        .gas(TGAS30)
        .with_signer(kid.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?
        .assert_success();
    let shop_after = shop.tokens().near_balance().fetch_from(&net).await?.total;
    assert_eq!(
        shop_after.as_yoctonear() - shop_before.as_yoctonear(),
        NearToken::from_millinear(400).as_yoctonear()
    );

    // paying someone off the allowlist fails
    contract
        .call_function(
            "spend",
            json!({"token_id": "1", "receiver_id": friend.account_id(), "amount": "1"}),
        )
        .transaction()
        .gas(TGAS30)
        .with_signer(kid.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?
        .assert_failure();

    // exceeding the 1 NEAR daily cap fails
    contract
        .call_function("spend", json!({"token_id": "1", "receiver_id": shop.account_id(), "amount": NearToken::from_millinear(700).as_yoctonear().to_string()}))
        .transaction()
        .gas(TGAS30)
        .with_signer(kid.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?
        .assert_failure();

    // kid hands the card to friend with a standard NEP-171 transfer
    contract
        .call_function(
            "nft_transfer",
            json!({"receiver_id": friend.account_id(), "token_id": "1"}),
        )
        .transaction()
        .deposit(NearToken::from_yoctonear(1))
        .gas(TGAS30)
        .with_signer(kid.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?
        .assert_success();
    let v = card(net.clone()).await;
    assert_eq!(v["owner_id"], friend.account_id().to_string());
    assert_eq!(
        v["available_now"],
        NearToken::from_millinear(600).as_yoctonear().to_string()
    );

    // the old holder can no longer spend
    contract
        .call_function(
            "spend",
            json!({"token_id": "1", "receiver_id": shop.account_id(), "amount": "1"}),
        )
        .transaction()
        .gas(TGAS30)
        .with_signer(kid.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?
        .assert_failure();

    // an unrestricted card paying a non-existent account: transfer fails, callback restores balance
    contract
        .call_function("mint_card", json!({"receiver_id": friend.account_id(), "title": "Open card", "daily_limit": "0", "allowlist": null, "expires_at_ms": null}))
        .transaction()
        .deposit(NearToken::from_near(1))
        .gas(TGAS30)
        .with_signer(mom.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?
        .assert_success();
    let before: Value = contract
        .call_function("get_card", json!({"token_id": "2"}))
        .read_only()
        .fetch_from(&net)
        .await?
        .data;
    let _ = contract
        .call_function("spend", json!({"token_id": "2", "receiver_id": "ghost.cards.sandbox", "amount": NearToken::from_millinear(100).as_yoctonear().to_string()}))
        .transaction()
        .gas(TGAS30)
        .with_signer(friend.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?;
    let after: Value = contract
        .call_function("get_card", json!({"token_id": "2"}))
        .read_only()
        .fetch_from(&net)
        .await?
        .data;
    assert_eq!(
        before["balance"], after["balance"],
        "failed payout must restore the card"
    );
    assert_eq!(after["spend_count"], "0");

    // anyone tops up
    contract
        .call_function("top_up", json!({"token_id": "1"}))
        .transaction()
        .deposit(NearToken::from_near(2))
        .gas(TGAS30)
        .with_signer(mom.account_id().clone(), signer.clone())
        .send_to(&net)
        .await?
        .assert_success();

    // wallets see a live SVG
    let tok: Value = contract
        .call_function("nft_token", json!({"token_id": "1"}))
        .read_only()
        .fetch_from(&net)
        .await?
        .data;
    assert!(
        tok["metadata"]["media"]
            .as_str()
            .unwrap()
            .starts_with("data:image/svg+xml;base64,")
    );
    let tokens: Value = contract
        .call_function(
            "nft_tokens_for_owner",
            json!({"account_id": friend.account_id()}),
        )
        .read_only()
        .fetch_from(&net)
        .await?
        .data;
    assert_eq!(tokens.as_array().unwrap().len(), 2);
    Ok(())
}
