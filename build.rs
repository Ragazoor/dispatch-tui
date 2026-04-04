use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn main() {
    // Must match the include_str!() calls in src/setup.rs lines 8-13.
    let plugin_files = [
        "plugin/.claude-plugin/plugin.json",
        "plugin/hooks/hooks.json",
        "plugin/hooks/scripts/task-status-hook",
        "plugin/hooks/scripts/task-usage-hook",
        "plugin/skills/wrap-up/SKILL.md",
        "plugin/commands/queue-plan.md",
    ];

    for path in &plugin_files {
        println!("cargo:rerun-if-changed={path}");
    }

    // Content hash ensures build-script output changes when files change,
    // which triggers Cargo to recompile the crate.
    let mut hasher = DefaultHasher::new();
    for path in &plugin_files {
        if let Ok(content) = std::fs::read_to_string(path) {
            content.hash(&mut hasher);
        }
    }
    println!("cargo:rustc-env=PLUGIN_CONTENT_HASH={}", hasher.finish());
}
