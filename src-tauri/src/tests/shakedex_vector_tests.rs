use crate::noncustodial::network::Network;
use crate::noncustodial::shakedex::script;
use crate::noncustodial::shakedex::template::{self, StepTemplate};
use crate::noncustodial::tx::output_address_from_string;
use serde_json::Value;

const VECTORS: &str = include_str!("../../tests/vectors/vectors.json");
fn sd() -> Value {
    serde_json::from_str::<Value>(VECTORS).unwrap()["shakedex"].clone()
}
fn h<const N: usize>(s: &str) -> [u8; N] {
    hex::decode(s).unwrap().try_into().unwrap()
}

#[test]
fn lock_script_and_address_match_hsd() {
    let v = sd();
    let pubkey: [u8; 33] = h(v["lockPub"].as_str().unwrap());
    assert_eq!(
        hex::encode(script::lock_script(&pubkey)),
        v["lockScript"].as_str().unwrap()
    );
    assert_eq!(
        script::lock_address(Network::Main, &pubkey).unwrap(),
        v["lockAddress"].as_str().unwrap()
    );
    assert!(script::is_lock_script_for(
        &script::lock_script(&pubkey),
        &pubkey
    ));
}

#[test]
fn live_dexreviews_lock_address_matches_its_coin() {
    let proof: Value = serde_json::from_str(include_str!(
        "../../tests/vectors/shakedex/proof_dexreviews.json"
    ))
    .unwrap();
    let coin: Value = serde_json::from_str(include_str!(
        "../../tests/vectors/shakedex/coin_dexreviews.json"
    ))
    .unwrap();
    let pubkey: [u8; 33] = h(proof["publicKey"].as_str().unwrap());
    assert_eq!(
        script::lock_address(Network::Main, &pubkey).unwrap(),
        coin["coin"]["address"].as_str().unwrap()
    );
}

#[test]
fn cargo_lock_has_no_hns_swap() {
    let lock = include_str!("../../Cargo.lock");
    assert!(
        !lock.contains("name = \"hns-swap\""),
        "ADR 0003: no hns-swap dependency"
    );
    assert!(!lock.contains("name = \"hns-wallet-shakedex\""), "ADR 0003");
}

#[test]
fn step_sighash_and_signature_match_hsd() {
    let v = sd();
    let pubkey: [u8; 33] = h(v["lockPub"].as_str().unwrap());
    let st = &v["step"];
    let t = StepTemplate {
        lock_outpoint: (h(v["lockCoin"]["hash"].as_str().unwrap()), 0),
        lock_value: v["lockCoin"]["value"].as_u64().unwrap(),
        lock_pubkey: &pubkey,
        payment: output_address_from_string(Network::Main, st["paymentAddr"].as_str().unwrap())
            .unwrap(),
        price: st["price"].as_u64().unwrap(),
        lock_time_secs: st["lockTimeSecs"].as_u64().unwrap(),
    };
    assert_eq!(
        template::encode_lock_time(t.lock_time_secs).unwrap() as u64,
        st["encodedLocktime"].as_u64().unwrap()
    );
    assert_eq!(
        hex::encode(template::step_sighash(&t).unwrap()),
        st["sighash"].as_str().unwrap()
    );
    let sig: [u8; 65] = h(st["signature"].as_str().unwrap());
    template::verify_step_signature(&t, &sig).unwrap();
    let mut tampered = t.clone();
    tampered.price += 1;
    assert!(template::verify_step_signature(&tampered, &sig).is_err());
    let mut wrong_type = sig;
    wrong_type[64] = 0x01;
    assert!(template::verify_step_signature(&t, &wrong_type).is_err());
}

fn vector_funding(f: &Value) -> crate::noncustodial::send::SpendableCoin {
    // Handshake does not byte-reverse hashes: displayTxid is the prevout hash.
    crate::noncustodial::send::SpendableCoin {
        txid: f["displayTxid"].as_str().unwrap().into(),
        vout: f["vout"].as_u64().unwrap() as u32,
        value: f["value"].as_u64().unwrap(),
        branch: f["branch"].as_u64().unwrap() as u32,
        child_index: f["index"].as_u64().unwrap() as u32,
    }
}

fn sign_with_known_mnemonic(plan: &crate::noncustodial::actions::DraftPlan) -> (String, String) {
    use crate::noncustodial::actions::sign_plan;
    use crate::noncustodial::session::SignerSession;
    let mut session = SignerSession::unlock(
        "p".into(),
        Network::Main,
        crate::tests::hsd_parity_tests::master_from_known_mnemonic(),
        60_000,
    );
    sign_plan(&mut session, plan).unwrap()
}

#[test]
fn purchase_plan_matches_hsd_signed_hex() {
    use crate::noncustodial::shakedex::listing_file::{ListingFile, PriceStep};
    use crate::noncustodial::shakedex::purchase::{build_purchase_plan, PurchaseInput};
    let v = sd();
    let st = &v["step"];
    let listing = ListingFile::for_tests(
        v["name"].as_str().unwrap(),
        h(v["lockCoin"]["hash"].as_str().unwrap()),
        v["lockCoin"]["index"].as_u64().unwrap() as u32,
        h(v["lockPub"].as_str().unwrap()),
        st["paymentAddr"].as_str().unwrap(),
        vec![PriceStep {
            price: st["price"].as_u64().unwrap(),
            lock_time: st["lockTimeSecs"].as_u64().unwrap(),
            signature: h(st["signature"].as_str().unwrap()),
            fee: 0,
        }],
    );
    let p = &v["purchase"];
    let funding = [vector_funding(&p["fundingInput"])];
    let res = build_purchase_plan(
        &PurchaseInput {
            network: Network::Main,
            account: 0,
            listing: &listing,
            step: 0,
            lock_value: v["lockCoin"]["value"].as_u64().unwrap(),
            name_height: v["height"].as_u64().unwrap() as u32,
            dest: output_address_from_string(Network::Main, p["destAddress"].as_str().unwrap())
                .unwrap(),
            market_fee: None,
            funding: &funding,
            change_address: p["changeAddress"].as_str().unwrap(),
            rate: 0,
            fixed_fee: None,
        }
        .with_fixed_fee_for_tests(p["fee"].as_u64().unwrap()),
    )
    .unwrap();
    let fee = p["fee"].as_u64().unwrap();
    let funding_value = p["fundingInput"]["value"].as_u64().unwrap();
    assert_eq!(res.fee, fee);
    assert_eq!(
        res.change,
        funding_value - st["price"].as_u64().unwrap() - fee
    );
    let (hex, txid) = sign_with_known_mnemonic(&res.plan);
    assert_eq!(hex, p["signedHex"].as_str().unwrap());
    assert_eq!(txid, p["txid"].as_str().unwrap());
    assert_eq!(res.txid, txid);
}

#[test]
fn purchase_finalize_plan_matches_hsd_signed_hex() {
    use crate::noncustodial::shakedex::purchase::{build_purchase_finalize_plan, FinalizeInput};
    let v = sd();
    let p = &v["purchase"];
    let f = &v["finalize"];
    let tc = &f["transferCoin"];
    let funding = [vector_funding(&f["fundingInput"])];
    let flags = f["flags"].as_u64().unwrap();
    assert!(flags <= 1, "flags carry only the weak bit");
    let res = build_purchase_finalize_plan(
        &FinalizeInput {
            network: Network::Main,
            account: 0,
            transfer_outpoint: (
                h(tc["hash"].as_str().unwrap()),
                tc["index"].as_u64().unwrap() as u32,
            ),
            transfer_value: tc["value"].as_u64().unwrap(),
            lock_pubkey: h(v["lockPub"].as_str().unwrap()),
            name: v["name"].as_str().unwrap(),
            name_height: v["height"].as_u64().unwrap() as u32,
            weak: flags == 1,
            claimed: f["claimed"].as_u64().unwrap() as u32,
            renewals: f["renewals"].as_u64().unwrap() as u32,
            renewal_block: h(f["renewalBlock"].as_str().unwrap()),
            dest_address: p["destAddress"].as_str().unwrap(),
            funding: &funding,
            change_address: p["changeAddress"].as_str().unwrap(),
            rate: 0,
            fixed_fee: None,
        }
        .with_fixed_fee_for_tests(f["fee"].as_u64().unwrap()),
    )
    .unwrap();
    let fee = f["fee"].as_u64().unwrap();
    assert_eq!(res.fee, fee);
    assert_eq!(
        res.change,
        f["fundingInput"]["value"].as_u64().unwrap() - fee
    );
    let (hex, txid) = sign_with_known_mnemonic(&res.plan);
    assert_eq!(hex, f["signedHex"].as_str().unwrap());
    assert_eq!(txid, f["txid"].as_str().unwrap());
    // The finalize spends the purchase's TRANSFER output.
    assert_eq!(tc["hash"].as_str().unwrap(), p["txid"].as_str().unwrap());
}

/// R1: the seller's price steps, signed by the R17 lock key from the test
/// phrase, equal hsd's signatures over the lock FINALIZE's output 0.
#[test]
fn seller_steps_match_hsd() {
    use crate::noncustodial::shakedex::lock_key::derive_lock_key;
    use crate::noncustodial::shakedex::sell::sign_step;
    let v = sd();
    let s = &v["sell"]["steps"];
    let key = derive_lock_key(
        &crate::tests::hsd_parity_tests::master_from_known_mnemonic(),
        Network::Main,
        0,
        v["name"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        hex::encode(key.pubkey),
        v["lockPub"].as_str().unwrap(),
        "the R17 key"
    );
    let lock = &s["lockCoin"];
    assert_eq!(
        lock["hash"], v["sell"]["lockFinalize"]["txid"],
        "steps spend the lock FINALIZE"
    );
    let payment =
        output_address_from_string(Network::Main, s["paymentAddr"].as_str().unwrap()).unwrap();
    let steps = s["data"].as_array().unwrap();
    assert_eq!(steps.len(), 3);
    for st in steps {
        let t = StepTemplate {
            lock_outpoint: (
                h(lock["hash"].as_str().unwrap()),
                lock["index"].as_u64().unwrap() as u32,
            ),
            lock_value: lock["value"].as_u64().unwrap(),
            lock_pubkey: &key.pubkey,
            payment: payment.clone(),
            price: st["price"].as_u64().unwrap(),
            lock_time_secs: st["lockTimeSecs"].as_u64().unwrap(),
        };
        assert_eq!(
            template::encode_lock_time(t.lock_time_secs).unwrap() as u64,
            st["encodedLocktime"].as_u64().unwrap()
        );
        assert_eq!(
            hex::encode(template::step_sighash(&t).unwrap()),
            st["sighash"].as_str().unwrap()
        );
        assert_eq!(
            hex::encode(sign_step(&key, &t).unwrap()),
            st["signature"].as_str().unwrap()
        );
    }
}

fn lock_finalize_input<'a>(
    v: &'a Value,
    transfer: &'a crate::noncustodial::send::SpendableCoin,
    funding: &'a [crate::noncustodial::send::SpendableCoin],
    rate: u64,
) -> crate::noncustodial::shakedex::sell::LockFinalizeInput<'a> {
    let lf = &v["sell"]["lockFinalize"];
    crate::noncustodial::shakedex::sell::LockFinalizeInput {
        network: Network::Main,
        account: 0,
        transfer,
        lock_pubkey: h(v["lockPub"].as_str().unwrap()),
        name: v["name"].as_str().unwrap(),
        name_height: v["height"].as_u64().unwrap() as u32,
        weak: false,
        claimed: 0,
        renewals: 0,
        renewal_block: h(lf["renewalBlock"].as_str().unwrap()),
        funding,
        change_address: lf["changeAddress"].as_str().unwrap(),
        rate,
        fixed_fee: None,
    }
}

/// R1 and R4: the FINALIZE of our TRANSFER coin into the lock equals hsd's
/// signed transaction, and at a fee rate it pays hsd's vsize times the rate.
#[test]
fn lock_finalize_matches_hsd_signed_hex() {
    use crate::noncustodial::shakedex::sell::build_lock_finalize_plan;
    let v = sd();
    let lf = &v["sell"]["lockFinalize"];
    let transfer = vector_funding(&lf["transferInput"]);
    let funding = [vector_funding(&lf["fundingInput"])];
    let fee = lf["fee"].as_u64().unwrap();
    let res = build_lock_finalize_plan(
        &lock_finalize_input(&v, &transfer, &funding, 0).with_fixed_fee_for_tests(fee),
    )
    .unwrap();
    assert_eq!(res.fee, fee);
    let (hex, txid) = sign_with_known_mnemonic(&res.plan);
    assert_eq!(hex, lf["signedHex"].as_str().unwrap());
    assert_eq!(txid, lf["txid"].as_str().unwrap());
    assert_eq!(res.txid, txid);

    let at_rate =
        build_lock_finalize_plan(&lock_finalize_input(&v, &transfer, &funding, 7)).unwrap();
    assert_eq!(
        at_rate.fee,
        lf["vsize"].as_u64().unwrap() * 7,
        "fee on hsd's vsize"
    );
}

fn cancel_input<'a>(
    v: &'a Value,
    funding: &'a [crate::noncustodial::send::SpendableCoin],
    rate: u64,
) -> crate::noncustodial::shakedex::cancel::CancelInput<'a> {
    let c = &v["sell"]["cancel"];
    let lock = &c["lockCoin"];
    crate::noncustodial::shakedex::cancel::CancelInput {
        network: Network::Main,
        account: 0,
        name: v["name"].as_str().unwrap(),
        name_height: v["height"].as_u64().unwrap() as u32,
        lock_outpoint: (
            h(lock["hash"].as_str().unwrap()),
            lock["index"].as_u64().unwrap() as u32,
        ),
        lock_value: lock["value"].as_u64().unwrap(),
        lock_pubkey: h(v["lockPub"].as_str().unwrap()),
        cancel_address: c["cancelAddress"].as_str().unwrap(),
        // The generator's `cancelDest = ring(0, 11)`.
        cancel_branch: 0,
        cancel_index: 11,
        funding,
        change_address: c["changeAddress"].as_str().unwrap(),
        rate,
        fixed_fee: None,
    }
}

/// R1 and R4: the cancel, built the way T5 will build it and accepted by the
/// signer (which re-derives the lock key and our cancel address from the
/// seed), equals hsd's signed transaction byte for byte, and at a fee rate it
/// pays hsd's vsize times the rate.
#[test]
fn cancel_matches_hsd_signed_hex() {
    use crate::noncustodial::shakedex::cancel::{build_cancel_plan, CANCEL_SIGHASH};
    let v = sd();
    let c = &v["sell"]["cancel"];
    assert_eq!(c["sighashType"].as_u64().unwrap() as u32, CANCEL_SIGHASH);
    assert_eq!(c["lockCoin"]["hash"], v["sell"]["lockFinalize"]["txid"]);
    let funding = [vector_funding(&c["fundingInput"])];
    let fee = c["fee"].as_u64().unwrap();
    let res =
        build_cancel_plan(&cancel_input(&v, &funding, 0).with_fixed_fee_for_tests(fee)).unwrap();
    assert_eq!(res.fee, fee);
    let (hex, txid) = sign_with_known_mnemonic(&res.plan);
    assert_eq!(hex, c["signedHex"].as_str().unwrap());
    assert_eq!(txid, c["txid"].as_str().unwrap());
    assert_eq!(res.txid, txid);

    let at_rate = build_cancel_plan(&cancel_input(&v, &funding, 7)).unwrap();
    assert_eq!(
        at_rate.fee,
        c["vsize"].as_u64().unwrap() * 7,
        "fee on hsd's vsize"
    );
}

/// R1 and R4: the cancel's FINALIZE out of the lock to our cancel address
/// equals hsd's signed transaction, and at a fee rate it pays hsd's vsize
/// times the rate.
#[test]
fn cancel_finalize_matches_hsd_signed_hex() {
    use crate::noncustodial::shakedex::cancel::{build_cancel_finalize_plan, CancelFinalizeInput};
    let v = sd();
    let c = &v["sell"]["cancel"];
    let f = &v["sell"]["cancelFinalize"];
    let cc = &f["cancelCoin"];
    assert_eq!(
        cc["hash"], c["txid"],
        "the FINALIZE spends the cancel's TRANSFER"
    );
    let funding = [vector_funding(&f["fundingInput"])];
    let flags = f["flags"].as_u64().unwrap();
    assert!(flags <= 1, "flags carry only the weak bit");
    let input = |rate| CancelFinalizeInput {
        network: Network::Main,
        account: 0,
        transfer_outpoint: (
            h(cc["hash"].as_str().unwrap()),
            cc["index"].as_u64().unwrap() as u32,
        ),
        transfer_value: cc["value"].as_u64().unwrap(),
        lock_pubkey: h(v["lockPub"].as_str().unwrap()),
        name: v["name"].as_str().unwrap(),
        name_height: v["height"].as_u64().unwrap() as u32,
        weak: flags == 1,
        claimed: f["claimed"].as_u64().unwrap() as u32,
        renewals: f["renewals"].as_u64().unwrap() as u32,
        renewal_block: h(f["renewalBlock"].as_str().unwrap()),
        dest_address: c["cancelAddress"].as_str().unwrap(),
        funding: &funding,
        change_address: f["changeAddress"].as_str().unwrap(),
        rate,
        fixed_fee: None,
    };
    let fee = f["fee"].as_u64().unwrap();
    let res = build_cancel_finalize_plan(&input(0).with_fixed_fee_for_tests(fee)).unwrap();
    assert_eq!(res.fee, fee);
    let (hex, txid) = sign_with_known_mnemonic(&res.plan);
    assert_eq!(hex, f["signedHex"].as_str().unwrap());
    assert_eq!(txid, f["txid"].as_str().unwrap());
    assert_eq!(res.txid, txid);

    let at_rate = build_cancel_finalize_plan(&input(7)).unwrap();
    assert_eq!(
        at_rate.fee,
        f["vsize"].as_u64().unwrap() * 7,
        "fee on hsd's vsize"
    );
}
