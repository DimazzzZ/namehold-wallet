use crate::db;

#[test]
fn test_list_assets_by_staked() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute(
        "INSERT INTO assets (tld, is_staked, status) VALUES ('a', 1, 'do_not_touch_staked')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assets (tld, is_staked, status) VALUES ('b', 0, 'not_started')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assets (tld, is_staked, status) VALUES ('c', 1, 'do_not_touch_staked')",
        [],
    )
    .unwrap();

    let staked = db::queries::list_assets(&conn, None, Some(true), None, None, None).unwrap();
    assert_eq!(staked.len(), 2);

    let unstaked = db::queries::list_assets(&conn, None, Some(false), None, None, None).unwrap();
    assert_eq!(unstaked.len(), 1);
}

#[test]
fn test_list_assets_combined_filters() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute("INSERT INTO assets (tld, is_staked, status, category) VALUES ('a', 0, 'not_started', 'Finance')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, is_staked, status, category) VALUES ('b', 0, 'finalized_owned', 'Finance')", []).unwrap();
    conn.execute("INSERT INTO assets (tld, is_staked, status, category) VALUES ('c', 1, 'do_not_touch_staked', 'Tech')", []).unwrap();

    let found = db::queries::list_assets(
        &conn,
        Some("not_started"),
        Some(false),
        Some("Finance"),
        None,
        None,
    )
    .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].tld, "a");
}

#[test]
fn test_list_assets_sort_by_category() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute(
        "INSERT INTO assets (tld, status, category) VALUES ('a', 'not_started', 'Zebra')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assets (tld, status, category) VALUES ('b', 'not_started', 'Apple')",
        [],
    )
    .unwrap();

    let sorted =
        db::queries::list_assets(&conn, None, None, None, Some("category"), Some("asc")).unwrap();
    assert_eq!(sorted[0].category.as_deref(), Some("Apple"));
}

#[test]
fn test_update_asset_tags_json() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('test', 'not_started')",
        [],
    )
    .unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();

    db::queries::update_asset(
        &conn,
        assets[0].id,
        None,
        None,
        Some(r#"["a","b","c"]"#),
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let asset = db::queries::get_asset(&conn, assets[0].id).unwrap();
    assert_eq!(asset.tags, vec!["a", "b", "c"]);
}

#[test]
fn test_bulk_update_empty_ids() {
    let conn = crate::tests::command_helpers::create_test_db();
    let updated = db::queries::bulk_update_status(&conn, &[], "not_started").unwrap();
    assert_eq!(updated, 0);
}

#[test]
fn test_create_batch_no_assets() {
    let conn = crate::tests::command_helpers::create_test_db();
    let id = db::queries::create_batch(&conn, "Empty", None, &[]).unwrap();
    let batch = db::queries::get_batch_with_assets(&conn, id).unwrap();
    assert_eq!(batch.assets.len(), 0);
}

#[test]
fn test_add_to_batch_duplicate() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('a', 'not_started')",
        [],
    )
    .unwrap();
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    let batch_id = db::queries::create_batch(&conn, "Test", None, &[assets[0].id]).unwrap();

    // Add same asset again - should not duplicate
    let added = db::queries::add_to_batch(&conn, batch_id, &[assets[0].id]).unwrap();
    assert_eq!(added, 0); // INSERT OR IGNORE

    let batch = db::queries::get_batch_with_assets(&conn, batch_id).unwrap();
    assert_eq!(batch.assets.len(), 1);
}

#[test]
fn test_wallet_snapshot_ordering() {
    let conn = crate::tests::command_helpers::create_test_db();
    db::queries::insert_wallet_snapshot(&conn, "primary", 100, None, 1, None).unwrap();
    db::queries::insert_wallet_snapshot(&conn, "primary", 200, None, 2, None).unwrap();
    db::queries::insert_wallet_snapshot(&conn, "primary", 300, None, 3, None).unwrap();

    let snapshots = db::queries::get_wallet_snapshots(&conn, 2).unwrap();
    assert_eq!(snapshots.len(), 2);
    // Should be ordered by id DESC (newest first)
    assert_eq!(snapshots[0]["balance"], 300);
    assert_eq!(snapshots[1]["balance"], 200);
}

#[test]
fn test_get_assets_by_tlds() {
    let conn = crate::tests::command_helpers::create_test_db();
    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('a', 'not_started')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('b', 'not_started')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assets (tld, status) VALUES ('c', 'not_started')",
        [],
    )
    .unwrap();

    let found =
        db::queries::get_assets_by_tlds(&conn, &["a".to_string(), "c".to_string()]).unwrap();
    assert_eq!(found.len(), 2);

    let found = db::queries::get_assets_by_tlds(&conn, &["nonexistent".to_string()]).unwrap();
    assert_eq!(found.len(), 0);
}

// ===========================================================================
// Branch-coverage tests for db::queries (row mappers, filters, upsert/empty
// paths). These target functions that were previously never invoked against
// non-empty result sets, so their row-mapping closures had zero coverage.
//
// These use the FULL migration set (`crate::db::migrations::run`) because the
// noncustodial tables (profiles, drafts, utxos, name states, bids) only exist
// after migrations 006+.
// ===========================================================================
mod branch_cov {
    use crate::db::queries::*;
    use rusqlite::{params, Connection};

    /// Fresh in-memory DB with the full migration chain applied.
    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        conn
    }

    fn seed_profile(conn: &Connection, id: &str) {
        insert_wallet_profile(
            conn,
            id,
            "Primary",
            "mnemonic_hot",
            "regtest",
            "xpubX",
            0,
            false,
        )
        .unwrap();
    }

    // --- settings / assets -------------------------------------------------

    #[test]
    fn get_settings_has_defaults() {
        let conn = db();
        let s = get_settings(&conn).unwrap();
        // After full migrations, node_rpc_url from 009 survives 010's cleanup.
        assert!(s.contains_key("node_rpc_url"));
    }

    #[test]
    fn set_setting_insert_then_conflict_update() {
        let conn = db();
        set_setting(&conn, "cov_key", "v1").unwrap();
        assert_eq!(get_settings(&conn).unwrap()["cov_key"], "v1");
        set_setting(&conn, "cov_key", "v2").unwrap();
        assert_eq!(get_settings(&conn).unwrap()["cov_key"], "v2");
    }

    #[test]
    fn list_assets_status_and_sort_desc_variants() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status, category) VALUES ('bravo','not_started','Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status, category) VALUES ('alpha','finalized_owned','A')",
            [],
        )
        .unwrap();
        let ns = list_assets(&conn, Some("not_started"), None, None, None, None).unwrap();
        assert_eq!(ns.len(), 1);
        assert_eq!(ns[0].tld, "bravo");
        let sorted = list_assets(&conn, None, None, None, Some("category"), Some("desc")).unwrap();
        assert_eq!(sorted[0].category.as_deref(), Some("Z"));
        let fallback =
            list_assets(&conn, None, None, None, Some("not_a_column"), Some("asc")).unwrap();
        assert_eq!(fallback[0].tld, "alpha");
    }

    #[test]
    fn get_asset_reads_row() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status, notes) VALUES ('x','not_started','n')",
            [],
        )
        .unwrap();
        let all = list_assets(&conn, None, None, None, None, None).unwrap();
        let a = get_asset(&conn, all[0].id).unwrap();
        assert_eq!(a.tld, "x");
        assert_eq!(a.notes.as_deref(), Some("n"));
    }

    #[test]
    fn update_asset_every_field_arm() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('u','not_started')",
            [],
        )
        .unwrap();
        let id = list_assets(&conn, None, None, None, None, None).unwrap()[0].id;
        update_asset(
            &conn,
            id,
            Some("finalized_owned"),
            Some("Finance"),
            Some(r#"["t"]"#),
            Some("note"),
            Some(42),
            Some("txhash"),
            Some("finhash"),
        )
        .unwrap();
        let a = get_asset(&conn, id).unwrap();
        assert_eq!(a.status.as_str(), "finalized_owned");
        assert_eq!(a.category.as_deref(), Some("Finance"));
        assert_eq!(a.notes.as_deref(), Some("note"));
        assert_eq!(a.hns_received, Some(42));
        assert_eq!(a.tags, vec!["t"]);
    }

    #[test]
    fn update_asset_no_fields_returns_early() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('u','not_started')",
            [],
        )
        .unwrap();
        let id = list_assets(&conn, None, None, None, None, None).unwrap()[0].id;
        update_asset(&conn, id, None, None, None, None, None, None, None).unwrap();
        let audit: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_log WHERE action='asset_update'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audit, 0);
    }

    #[test]
    fn bulk_update_status_and_tags_nonempty() {
        let conn = db();
        for t in ["a", "b"] {
            conn.execute(
                "INSERT INTO assets (tld, status) VALUES (?1,'not_started')",
                params![t],
            )
            .unwrap();
        }
        let ids: Vec<i64> = list_assets(&conn, None, None, None, None, None)
            .unwrap()
            .iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(
            bulk_update_status(&conn, &ids, "finalized_owned").unwrap(),
            2
        );
        assert_eq!(bulk_update_tags(&conn, &ids, r#"["x"]"#).unwrap(), 2);
        let a = get_asset(&conn, ids[0]).unwrap();
        assert_eq!(a.status.as_str(), "finalized_owned");
        assert_eq!(a.tags, vec!["x"]);
    }

    #[test]
    fn set_asset_status_by_tld_updates_and_noops() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('here','not_started')",
            [],
        )
        .unwrap();
        set_asset_status_by_tld(&conn, "here", "namebase_transfer_requested").unwrap();
        set_asset_status_by_tld(&conn, "missing", "namebase_transfer_requested").unwrap();
        let a = &list_assets(&conn, None, None, None, None, None).unwrap()[0];
        assert_eq!(a.status.as_str(), "namebase_transfer_requested");
    }

    #[test]
    fn delete_asset_removes_row_and_audits() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('gone','not_started')",
            [],
        )
        .unwrap();
        let id = list_assets(&conn, None, None, None, None, None).unwrap()[0].id;
        delete_asset(&conn, id).unwrap();
        assert_eq!(
            list_assets(&conn, None, None, None, None, None)
                .unwrap()
                .len(),
            0
        );
    }

    // --- batches -----------------------------------------------------------

    #[test]
    fn batch_list_get_update_add_remove_delete() {
        let conn = db();
        for t in ["a", "b", "c"] {
            conn.execute(
                "INSERT INTO assets (tld, status) VALUES (?1,'not_started')",
                params![t],
            )
            .unwrap();
        }
        let ids: Vec<i64> = list_assets(&conn, None, None, None, None, None)
            .unwrap()
            .iter()
            .map(|a| a.id)
            .collect();
        let bid = create_batch(&conn, "B", Some("d"), &ids[..2]).unwrap();

        let batches = list_batches(&conn).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].name, "B");
        assert_eq!(batches[0].asset_count, Some(2));

        let bw = get_batch_with_assets(&conn, bid).unwrap();
        assert_eq!(bw.assets.len(), 2);
        assert_eq!(bw.description.as_deref(), Some("d"));

        update_batch(&conn, bid, Some("B2"), Some("d2"), Some("in_progress")).unwrap();
        let b = &list_batches(&conn).unwrap()[0];
        assert_eq!(b.name, "B2");
        assert_eq!(b.status.as_str(), "in_progress");

        update_batch(&conn, bid, None, None, None).unwrap();

        assert_eq!(add_to_batch(&conn, bid, &[ids[2]]).unwrap(), 1);
        assert_eq!(add_to_batch(&conn, bid, &[ids[2]]).unwrap(), 0);
        assert_eq!(get_batch_with_assets(&conn, bid).unwrap().assets.len(), 3);
        assert_eq!(remove_from_batch(&conn, bid, &[ids[2]]).unwrap(), 1);
        assert_eq!(get_batch_with_assets(&conn, bid).unwrap().assets.len(), 2);

        delete_batch(&conn, bid).unwrap();
        assert_eq!(list_batches(&conn).unwrap().len(), 0);
    }

    // --- dashboard / audit -------------------------------------------------

    #[test]
    fn dashboard_stats_counts_and_status_map() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status, is_staked) VALUES ('a','not_started',0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status, is_staked) VALUES ('b','finalized_owned',1)",
            [],
        )
        .unwrap();
        let stats = get_dashboard_stats(&conn).unwrap();
        assert_eq!(stats["total"], 2);
        assert_eq!(stats["staked"], 1);
        assert_eq!(stats["unstaked"], 1);
        assert_eq!(stats["status_counts"]["not_started"], 1);
        assert_eq!(stats["status_counts"]["finalized_owned"], 1);
    }

    #[test]
    fn recent_audit_log_maps_null_and_present_columns() {
        let conn = db();
        conn.execute(
            "INSERT INTO audit_log (action, entity, entity_id, detail) VALUES ('a1','asset',7,'d')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO audit_log (action) VALUES ('a2')", [])
            .unwrap();
        let entries = get_recent_audit_log(&conn, 10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["action"], "a2");
        assert!(entries[0]["entity"].is_null());
        assert!(entries[0]["entity_id"].is_null());
        assert!(entries[0]["detail"].is_null());
        assert_eq!(entries[1]["action"], "a1");
        assert_eq!(entries[1]["entity"], "asset");
        assert_eq!(entries[1]["entity_id"], 7);
    }

    // --- wallet snapshots / addresses --------------------------------------

    #[test]
    fn snapshot_latest_and_list_map_null_address() {
        let conn = db();
        insert_wallet_snapshot(&conn, "primary", 100, Some("rs1qA"), 3, Some("{}")).unwrap();
        insert_wallet_snapshot(&conn, "primary", 200, None, 4, None).unwrap();
        let latest = get_latest_wallet_snapshot(&conn).unwrap().unwrap();
        assert_eq!(latest["balance"], 200);
        assert!(latest["address"].is_null());
        assert_eq!(latest["name_count"], 4);

        let list = get_wallet_snapshots(&conn, 10).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["balance"], 200);
        assert_eq!(list[1]["address"], "rs1qA");
    }

    #[test]
    fn known_wallet_addresses_distinct_nonempty_newest_first() {
        let conn = db();
        insert_wallet_snapshot(&conn, "primary", 1, Some("rs1qA"), 0, None).unwrap();
        insert_wallet_snapshot(&conn, "primary", 2, Some(""), 0, None).unwrap();
        insert_wallet_snapshot(&conn, "primary", 3, Some("rs1qB"), 0, None).unwrap();
        insert_wallet_snapshot(&conn, "primary", 4, Some("rs1qB"), 0, None).unwrap();
        let addrs = get_known_wallet_addresses(&conn, 10).unwrap();
        assert_eq!(addrs, vec!["rs1qB".to_string(), "rs1qA".to_string()]);
    }

    #[test]
    fn replace_and_read_wallet_addresses_for_wallet() {
        let conn = db();
        let n = replace_wallet_addresses(
            &conn,
            "w1",
            &["rs1qA".to_string(), "  ".to_string(), "rs1qB".to_string()],
        )
        .unwrap();
        assert_eq!(n, 2);
        let n2 = replace_wallet_addresses(&conn, "w1", &["rs1qA".to_string()]).unwrap();
        assert_eq!(n2, 1);
        let got = get_wallet_addresses_for_wallet(&conn, "w1", 10).unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.contains(&"rs1qA".to_string()));
        assert!(got.contains(&"rs1qB".to_string()));
    }

    #[test]
    fn inventory_tlds_sorted() {
        let conn = db();
        for t in ["gamma", "alpha", "beta"] {
            conn.execute(
                "INSERT INTO assets (tld, status) VALUES (?1,'not_started')",
                params![t],
            )
            .unwrap();
        }
        assert_eq!(
            get_inventory_tlds(&conn).unwrap(),
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
    }

    // --- repair candidates -------------------------------------------------

    #[test]
    fn repair_candidates_and_recently_synced() {
        let conn = db();
        seed_profile(&conn, "p1");
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('never','not_started')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status, last_synced_at) VALUES ('old','not_started', datetime('now','-10 days'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status, last_synced_at) VALUES ('fresh','not_started', datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tracked_name_states (wallet_profile_id, name, name_hash_hex, state)
             VALUES ('p1','tracked','hh','CLOSED')",
            [],
        )
        .unwrap();

        let cands = list_repair_candidates(&conn, "p1", 10, 24).unwrap();
        assert!(cands.contains(&"never".to_string()));
        assert!(cands.contains(&"old".to_string()));
        assert!(cands.contains(&"tracked".to_string()));
        assert!(!cands.contains(&"fresh".to_string()));

        assert_eq!(
            count_repair_candidates(&conn, "p1", 24).unwrap() as usize,
            cands.len()
        );

        let recent = list_recently_synced_tlds(&conn, 24).unwrap();
        assert_eq!(recent, vec!["fresh".to_string()]);
    }

    #[test]
    fn mark_finalized_and_touch_synced() {
        let conn = db();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('own','not_started')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('staked','do_not_touch_staked')",
            [],
        )
        .unwrap();
        mark_asset_finalized_owned(&conn, "own", Some("CLOSED")).unwrap();
        mark_asset_finalized_owned(&conn, "staked", Some("CLOSED")).unwrap();
        let assets = list_assets(&conn, None, None, None, None, None).unwrap();
        let own = assets.iter().find(|a| a.tld == "own").unwrap();
        let staked = assets.iter().find(|a| a.tld == "staked").unwrap();
        assert_eq!(own.status.as_str(), "finalized_owned");
        assert_eq!(staked.status.as_str(), "do_not_touch_staked");
        touch_asset_synced(&conn, "own").unwrap();
        touch_asset_synced(&conn, "not_in_inventory").unwrap();
    }

    // --- wallet profiles ---------------------------------------------------

    #[test]
    fn profile_list_and_get_null_and_active() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_wallet_profile(
            &conn,
            "p2",
            "Watch",
            "watch_only_xpub",
            "regtest",
            "xpubW",
            0,
            true,
        )
        .unwrap();
        set_active_profile(&conn, "p1").unwrap();

        let list = list_wallet_profiles(&conn).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list.iter().filter(|p| p.active).count(), 1);
        let watch = list.iter().find(|p| p.id == "p2").unwrap();
        assert!(watch.watch_only);
        assert!(!watch.has_passphrase);

        assert!(get_wallet_profile(&conn, "p1").unwrap().unwrap().active);
        assert!(get_wallet_profile(&conn, "nope").unwrap().is_none());
    }

    #[test]
    fn profile_change_depth_and_stamp_explorer() {
        let conn = db();
        seed_profile(&conn, "p1");
        update_profile_change_depth(&conn, "p1", 7).unwrap();
        update_profile_change_depth(&conn, "p1", 3).unwrap();
        stamp_explorer_sync(&conn, "p1").unwrap();
        let p = get_wallet_profile(&conn, "p1").unwrap().unwrap();
        assert_eq!(p.change_depth, 7);
        assert!(p.last_explorer_sync_at.is_some());
    }

    #[test]
    fn delete_wallet_profile_cascades_rows() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_wallet_secret(&conn, "p1", &[1, 2, 3], "argon2id", "fp").unwrap();
        delete_wallet_profile(&conn, "p1").unwrap();
        assert!(get_wallet_profile(&conn, "p1").unwrap().is_none());
        assert_eq!(get_wallet_secret_meta(&conn, "p1").unwrap(), None);
    }

    // --- tx drafts ---------------------------------------------------------

    fn insert_basic_draft(conn: &Connection, id: &str, action: &str, name: &str) {
        insert_tx_draft(
            conn,
            id,
            "p1",
            action,
            "00",
            "{}",
            &format!(r#"{{"action":"{action}","name":"{name}"}}"#),
        )
        .unwrap();
    }

    #[test]
    fn tx_draft_row_mapper_and_lists() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_basic_draft(&conn, "d1", "send_hns", "");
        let d = get_tx_draft(&conn, "d1").unwrap().unwrap();
        assert_eq!(d.status, "draft");
        assert!(d.signed_tx_hex.is_none());
        assert!(d.txid.is_none());
        assert!(d.confirmation_height.is_none());
        assert!(d.error_message.is_none());

        let list = list_tx_drafts(&conn, "p1").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "d1");

        assert!(get_tx_draft(&conn, "missing").unwrap().is_none());
    }

    #[test]
    fn draft_status_transitions_and_ages() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_basic_draft(&conn, "d1", "send_hns", "");
        update_tx_draft_signed(&conn, "d1", "aabb", r#"{"action":"send_hns"}"#).unwrap();
        update_tx_draft_status(&conn, "d1", "broadcasted", None, Some("tx1")).unwrap();
        let d = get_tx_draft(&conn, "d1").unwrap().unwrap();
        assert_eq!(d.status, "broadcasted");
        assert_eq!(d.txid.as_deref(), Some("tx1"));
        assert_eq!(d.signed_tx_hex.as_deref(), Some("aabb"));

        update_tx_draft_confirmation(&conn, "d1", 500, None).unwrap();
        let d = get_tx_draft(&conn, "d1").unwrap().unwrap();
        assert_eq!(d.status, "confirmed");
        assert_eq!(d.confirmation_height, Some(500));

        revert_tx_draft_to_broadcasted(&conn, "d1", "reorg").unwrap();
        let d = get_tx_draft(&conn, "d1").unwrap().unwrap();
        assert_eq!(d.status, "broadcasted");
        assert!(d.confirmation_height.is_none());
        assert_eq!(d.error_message.as_deref(), Some("reorg"));

        assert!(draft_age_secs(&conn, "d1").unwrap() >= 0);
        assert!(draft_updated_age_secs(&conn, "d1").unwrap() >= 0);
    }

    #[test]
    fn delete_tx_draft_refuses_broadcasted_and_missing() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_basic_draft(&conn, "d1", "send_hns", "");
        update_tx_draft_status(&conn, "d1", "broadcasted", None, Some("tx1")).unwrap();
        assert!(delete_tx_draft(&conn, "d1").is_err());
        assert!(delete_tx_draft(&conn, "nope").is_err());

        insert_basic_draft(&conn, "d2", "send_hns", "");
        delete_tx_draft(&conn, "d2").unwrap();
        assert!(get_tx_draft(&conn, "d2").unwrap().is_none());
    }

    #[test]
    fn drafts_awaiting_confirmation_filters() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_basic_draft(&conn, "b", "send_hns", "");
        update_tx_draft_status(&conn, "b", "broadcasted", None, Some("txb")).unwrap();
        insert_basic_draft(&conn, "c", "send_hns", "");
        update_tx_draft_status(&conn, "c", "broadcasted", None, Some("txc")).unwrap();
        update_tx_draft_confirmation(&conn, "c", 100, None).unwrap();
        insert_basic_draft(&conn, "deep", "send_hns", "");
        update_tx_draft_status(&conn, "deep", "broadcasted", None, Some("txd")).unwrap();
        update_tx_draft_confirmation(&conn, "deep", 1, None).unwrap();
        insert_basic_draft(&conn, "plain", "send_hns", "");

        let awaiting = list_drafts_awaiting_confirmation(&conn, "p1", 100, 6).unwrap();
        let ids: Vec<&str> = awaiting.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));
        assert!(!ids.contains(&"deep"));
        assert!(!ids.contains(&"plain"));
    }

    #[test]
    fn get_draft_status_by_txid_some_and_none() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_basic_draft(&conn, "d1", "send_hns", "");
        update_tx_draft_status(&conn, "d1", "broadcasted", None, Some("txX")).unwrap();
        assert_eq!(
            get_draft_status_by_txid(&conn, "p1", "txX")
                .unwrap()
                .as_deref(),
            Some("broadcasted")
        );
        assert!(get_draft_status_by_txid(&conn, "p1", "nope")
            .unwrap()
            .is_none());
    }

    #[test]
    fn has_pending_draft_variants() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_basic_draft(&conn, "open1", "open", "wanted");
        assert!(has_pending_draft_for_name(&conn, "p1", "open", "wanted").unwrap());
        assert!(!has_pending_draft_for_name(&conn, "p1", "open", "other").unwrap());

        insert_tx_draft(
            &conn,
            "bb",
            "p1",
            "batch-bid",
            "00",
            "",
            r#"{"action":"batch-bid","nameList":["inbatch"]}"#,
        )
        .unwrap();
        assert!(has_pending_bid_draft_for_name(&conn, "p1", "inbatch").unwrap());
        assert!(!has_pending_bid_draft_for_name(&conn, "p1", "notbid").unwrap());
    }

    // --- profile addresses / receive addresses -----------------------------

    #[test]
    fn profile_addresses_and_receive_rows() {
        let conn = db();
        seed_profile(&conn, "p1");
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index, address, script_pubkey_hex, public_key_hex, created_at)
             VALUES ('p1',0,0,0,'rs1qr0','0014','02','2026-01-01T00:00:00'),
                    ('p1',0,0,1,'rs1qr1','0014','02','2026-01-01T00:01:00'),
                    ('p1',0,1,0,'rs1qch','0014','02','2026-01-01T00:02:00')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tracked_utxos (txid, vout, wallet_profile_id, address, script_pubkey_hex, value_doos, covenant_type, spend_class)
             VALUES ('t',0,'p1','rs1qr0','0014',10,0,'liquid_hns')",
            [],
        )
        .unwrap();
        let all = get_profile_addresses(&conn, "p1").unwrap();
        assert_eq!(
            all,
            vec![
                "rs1qr0".to_string(),
                "rs1qr1".to_string(),
                "rs1qch".to_string()
            ]
        );

        let recv = list_receive_addresses(&conn, "p1", 0).unwrap();
        assert_eq!(recv.len(), 2);
        assert!(recv[0].used);
        assert!(!recv[1].used);
    }

    // --- name states / owned names -----------------------------------------

    #[test]
    fn upsert_owned_name_insert_and_conflict_update() {
        use crate::hsd::types::HsdName;
        let conn = db();
        seed_profile(&conn, "p1");
        let name: HsdName = serde_json::from_value(serde_json::json!({
            "name": "owned",
            "nameHash": "nh",
            "state": "CLOSED",
            "height": 10,
            "renewal": 1000,
            "owner": {"hash": "ownA", "index": 0},
            "registered": true,
            "expired": false,
        }))
        .unwrap();
        upsert_owned_name(&conn, "p1", &name, "ownA", 0, "rs1qOwnerA").unwrap();
        upsert_owned_name(&conn, "p1", &name, "ownB", 0, "rs1qOwnerB").unwrap();
        let row = get_tracked_name_state(&conn, "p1", "owned")
            .unwrap()
            .unwrap();
        assert_eq!(row.owner_address.as_deref(), Some("rs1qOwnerB"));

        let owned = read_owned_names_explorer(&conn, "p1").unwrap();
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0]["name"], "owned");
        assert_eq!(owned[0]["owner_address"], "rs1qOwnerB");
        assert_eq!(owned[0]["registered"], true);

        assert_eq!(
            list_tracked_name_names(&conn, "p1").unwrap(),
            vec!["owned".to_string()]
        );
    }

    #[test]
    fn get_tracked_name_state_none_for_missing() {
        let conn = db();
        seed_profile(&conn, "p1");
        assert!(get_tracked_name_state(&conn, "p1", "absent")
            .unwrap()
            .is_none());
    }

    // --- name coins / covenant utxos ---------------------------------------

    fn seed_addr(conn: &Connection, branch: i64, idx: i64, addr: &str) {
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index, address, script_pubkey_hex, public_key_hex)
             VALUES ('p1',0,?1,?2,?3,'0014','02')",
            params![branch, idx, addr],
        )
        .unwrap();
    }

    fn seed_cov_utxo(
        conn: &Connection,
        txid: &str,
        vout: i64,
        addr: &str,
        cov: i64,
        class: &str,
        cov_json: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO tracked_utxos
                (txid, vout, wallet_profile_id, address, script_pubkey_hex, value_doos, covenant_type, covenant_json, spend_class)
             VALUES (?1,?2,'p1',?3,'0014',1000,?4,?5,?6)",
            params![txid, vout, addr, cov, cov_json, class],
        )
        .unwrap();
    }

    #[test]
    fn get_name_coin_some_and_none() {
        let conn = db();
        seed_profile(&conn, "p1");
        seed_addr(&conn, 0, 0, "rs1qown");
        seed_cov_utxo(&conn, "owntx", 0, "rs1qown", 6, "name_control", None);
        conn.execute(
            "INSERT INTO tracked_name_states (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, height)
             VALUES ('p1','mine','nh','CLOSED','owntx',0,42)",
            [],
        )
        .unwrap();
        let coin = get_name_coin(&conn, "p1", "mine").unwrap().unwrap();
        assert_eq!(coin.txid, "owntx");
        assert_eq!(coin.covenant_type, 6);
        assert_eq!(coin.name_height, Some(42));
        assert!(get_name_coin(&conn, "p1", "absent").unwrap().is_none());
    }

    #[test]
    fn find_unspent_covenant_utxo_match_and_none() {
        let conn = db();
        seed_profile(&conn, "p1");
        seed_addr(&conn, 0, 0, "rs1qbid");
        let cj = r#"{"type":3,"action":"BID","items":["AABB","","","blindhex"]}"#;
        seed_cov_utxo(&conn, "bidtx", 0, "rs1qbid", 3, "name_lockup", Some(cj));
        let found = find_unspent_covenant_utxo(&conn, "p1", "rs1qbid", 3, "name", "aabb").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().txid, "bidtx");
        assert!(
            find_unspent_covenant_utxo(&conn, "p1", "rs1qbid", 3, "name", "ffff")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn find_unspent_covenant_utxo_lone_unknown_fallback() {
        let conn = db();
        seed_profile(&conn, "p1");
        seed_addr(&conn, 0, 0, "rs1qsole");
        seed_cov_utxo(&conn, "soletx", 0, "rs1qsole", 3, "name_lockup", None);
        let found =
            find_unspent_covenant_utxo(&conn, "p1", "rs1qsole", 3, "name", "whatever").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().txid, "soletx");
    }

    #[test]
    fn find_unspent_covenant_utxos_by_name_hash_scans_all() {
        let conn = db();
        seed_profile(&conn, "p1");
        seed_addr(&conn, 0, 0, "rs1qa");
        seed_addr(&conn, 0, 1, "rs1qb");
        let cj_match = r#"{"type":3,"items":["AABB","","",""]}"#;
        let cj_other = r#"{"type":3,"items":["CCDD","","",""]}"#;
        seed_cov_utxo(&conn, "m1", 0, "rs1qa", 3, "name_lockup", Some(cj_match));
        seed_cov_utxo(&conn, "m2", 0, "rs1qb", 3, "name_lockup", Some(cj_other));
        let hits = find_unspent_covenant_utxos_by_name_hash(&conn, "p1", 3, "aabb").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].txid, "m1");
    }

    #[test]
    fn covenant_item_hex_variants() {
        let cj = r#"{"items":["AABB","cc"]}"#;
        assert_eq!(covenant_item_hex(Some(cj), 0).as_deref(), Some("aabb"));
        assert_eq!(covenant_item_hex(Some(cj), 1).as_deref(), Some("cc"));
        assert!(covenant_item_hex(Some(cj), 9).is_none());
        assert!(covenant_item_hex(Some(r#"{"items":[""]}"#), 0).is_none());
        assert!(covenant_item_hex(None, 0).is_none());
        assert!(covenant_item_hex(Some("not json"), 0).is_none());
    }

    #[test]
    fn list_unspent_wallet_name_hashes_collapses_and_rawname() {
        let conn = db();
        seed_profile(&conn, "p1");
        let open_cj = r#"{"type":2,"items":["AABB","","6e616d65"]}"#;
        let reveal_cj = r#"{"type":4,"items":["AABB","noncehex"]}"#;
        seed_cov_utxo(&conn, "o", 0, "rs1qx", 2, "name_lockup", Some(open_cj));
        seed_cov_utxo(&conn, "r", 0, "rs1qx", 4, "name_control", Some(reveal_cj));
        seed_cov_utxo(
            &conn,
            "bad",
            0,
            "rs1qx",
            2,
            "name_lockup",
            Some(r#"{"type":2,"items":[]}"#),
        );
        let hashes = list_unspent_wallet_name_hashes(&conn, "p1").unwrap();
        assert_eq!(hashes.len(), 1);
        assert_eq!(hashes[0].name_hash_hex, "aabb");
        assert_eq!(hashes[0].raw_name_hex.as_deref(), Some("6e616d65"));
    }

    // --- bid commitments ---------------------------------------------------

    #[test]
    fn bid_commitment_insert_conflict_and_list_and_deadlines() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_bid_commitment(
            &conn, "p1", "n1", "h1", "rs1q", 0, 0, 100, 200, "nn1", "bl1",
        )
        .unwrap();
        assert!(insert_bid_commitment(
            &conn, "p1", "n1", "h1", "rs1q", 0, 0, 100, 200, "nn1", "bl1"
        )
        .is_err());
        assert!(bid_commitment_exists(&conn, "p1", "n1", "bl1").unwrap());
        assert!(!bid_commitment_exists(&conn, "p1", "n1", "other").unwrap());

        let list = list_bid_commitments(&conn, "p1").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "n1");
        assert_eq!(list[0].bid_value_doos, 100);
        assert!(list[0].reveal_end_height.is_none());

        set_reveal_end_height(&conn, "p1", "bl1", 555).unwrap();
        let deadlines = list_pending_reveal_deadlines(&conn).unwrap();
        assert_eq!(deadlines.len(), 1);
        assert_eq!(deadlines[0].0, "p1");
        assert_eq!(deadlines[0].1, "n1");
        assert_eq!(deadlines[0].2, 555);

        set_bid_txid(&conn, "p1", "bl1", "bidtx").unwrap();
        set_bid_reveal_txid(&conn, "p1", "n1", "revtx").unwrap();
        assert_eq!(list_pending_reveal_deadlines(&conn).unwrap().len(), 0);
        let after = list_bid_commitments(&conn, "p1").unwrap();
        assert_eq!(after[0].bid_txid.as_deref(), Some("bidtx"));
        assert_eq!(after[0].reveal_txid.as_deref(), Some("revtx"));
    }

    #[test]
    fn auction_position_names_from_drafts_and_bids_excludes_owned() {
        let conn = db();
        seed_profile(&conn, "p1");
        insert_basic_draft(&conn, "op", "open", "opening");
        update_tx_draft_status(&conn, "op", "broadcasted", None, Some("txop")).unwrap();
        insert_basic_draft(&conn, "plain", "open", "notqueued");
        insert_bid_commitment(&conn, "p1", "bidname", "h", "rs1q", 0, 0, 1, 2, "n", "b").unwrap();
        seed_addr(&conn, 0, 0, "rs1qown");
        seed_cov_utxo(&conn, "owntx", 0, "rs1qown", 6, "name_control", None);
        conn.execute(
            "INSERT INTO tracked_name_states (wallet_profile_id, name, name_hash_hex, state, owner_txid, owner_vout, height)
             VALUES ('p1','ownedname','nh','CLOSED','owntx',0,1)",
            [],
        )
        .unwrap();
        insert_bid_commitment(
            &conn,
            "p1",
            "ownedname",
            "h2",
            "rs1q",
            0,
            0,
            1,
            2,
            "n2",
            "b2",
        )
        .unwrap();

        let names = auction_position_names(&conn, "p1").unwrap();
        assert!(names.contains(&"opening".to_string()));
        assert!(names.contains(&"bidname".to_string()));
        assert!(!names.contains(&"notqueued".to_string()));
        assert!(!names.contains(&"ownedname".to_string()));
    }
}

// Additional narrowly-scoped tests to hit remaining reachable branches.
mod branch_cov_extra {
    use crate::db::queries::*;
    use rusqlite::Connection;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        conn
    }

    /// Hit the `state.unwrap_or_else(|| "UNKNOWN".to_string())` branch in
    /// `upsert_owned_name` — happens only when the caller passes an HsdName
    /// with `state: None`.
    #[test]
    fn upsert_owned_name_state_none_defaults_to_unknown() {
        use crate::hsd::types::HsdName;
        let conn = db();
        insert_wallet_profile(&conn, "p1", "P", "mnemonic_hot", "regtest", "x", 0, false).unwrap();
        let name: HsdName = serde_json::from_value(serde_json::json!({
            "name": "noState",
            "nameHash": "nh",
            "owner": {"hash": "own", "index": 0},
        }))
        .unwrap();
        // state is None -> fallback "UNKNOWN"
        upsert_owned_name(&conn, "p1", &name, "own", 0, "rs1qA").unwrap();
        let got = get_tracked_name_state(&conn, "p1", "noState")
            .unwrap()
            .unwrap();
        assert_eq!(got.state.as_deref(), Some("UNKNOWN"));
    }

    /// Hit the "draft has no `name` field in summary_json" `continue` branch
    /// in `auction_position_names` — a queued draft with only `nameList`
    /// (no `name`).
    #[test]
    fn auction_position_names_skips_draft_without_name_field() {
        let conn = db();
        insert_wallet_profile(&conn, "p1", "P", "mnemonic_hot", "regtest", "x", 0, false).unwrap();
        // A broadcasted "open" draft whose summary has NO `name` key.
        insert_tx_draft(
            &conn,
            "opnn",
            "p1",
            "open",
            "00",
            "{}",
            r#"{"action":"open","other":"field"}"#,
        )
        .unwrap();
        update_tx_draft_status(&conn, "opnn", "broadcasted", None, Some("txopnn")).unwrap();
        // Even though the draft is in-flight, no "name" is inserted.
        let names = auction_position_names(&conn, "p1").unwrap();
        assert!(
            names.is_empty(),
            "draft without a name field must be skipped"
        );
    }
}
