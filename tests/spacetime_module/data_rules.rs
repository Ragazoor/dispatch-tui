use super::*;

/// A connection stamps the rules version, so a store upgraded by a plain
/// publish passes the gate: nothing in production calls `set_schema_version`.
#[test]
fn a_board_connecting_to_an_upgraded_store_passes_the_version_gate() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("module-test");
    let scratch = tempfile::tempdir().expect("temp dir");
    let previous = committed_module(scratch.path());
    let first = instance.publish(&previous, Some(&scratch_target_dir()));
    assert!(first.status.success(), "{}", describe(&first));
    let migrated = instance.publish(&module_path(), None);
    assert!(migrated.status.success(), "{}", describe(&migrated));

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let connector =
            SpacetimeSdkConnector::new(instance.database(), Arc::new(SharedRows::new()));
        connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("the gate must accept the module it was built with: {e}"));
    });
    let json = instance.sql("SELECT version FROM rules_version");
    assert!(
        json.contains(&format!(
            "[{}]",
            dispatch_tui::sync::rules_gate::BOARD_RULES_VERSION
        )),
        "the connection stamped the module's rules version: {json}"
    );
}
