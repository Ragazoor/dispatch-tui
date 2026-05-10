#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[tokio::test]
async fn security_alerts_round_trip() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};

    let db = in_memory_db().await;
    let now = chrono::Utc::now();

    let alerts = vec![
        SecurityAlert {
            number: 1,
            repo: "acme/app".to_string(),
            severity: AlertSeverity::Critical,
            kind: AlertKind::Dependabot,
            title: "CVE-2024-1234".to_string(),
            package: Some("lodash".to_string()),
            vulnerable_range: Some("< 4.17.21".to_string()),
            fixed_version: Some("4.17.21".to_string()),
            cvss_score: Some(9.8),
            url: "https://github.com/acme/app/security/dependabot/1".to_string(),
            created_at: now,
            state: "open".to_string(),
            description: "Prototype pollution".to_string(),
        },
        SecurityAlert {
            number: 2,
            repo: "acme/app".to_string(),
            severity: AlertSeverity::Low,
            kind: AlertKind::CodeScanning,
            title: "SQL injection".to_string(),
            package: None,
            vulnerable_range: None,
            fixed_version: None,
            cvss_score: None,
            url: "https://github.com/acme/app/security/code-scanning/2".to_string(),
            created_at: now,
            state: "open".to_string(),
            description: "Potential SQL injection".to_string(),
        },
    ];

    db.save_security_alerts(&alerts).await.unwrap();
    let loaded = db.load_security_alerts().await.unwrap();

    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].number, 1);
    assert_eq!(loaded[0].repo, "acme/app");
    assert_eq!(loaded[0].severity, AlertSeverity::Critical);
    assert_eq!(loaded[0].kind, AlertKind::Dependabot);
    assert_eq!(loaded[0].package.as_deref(), Some("lodash"));
    assert_eq!(loaded[0].cvss_score, Some(9.8));
    assert_eq!(loaded[0].description, "Prototype pollution");

    assert_eq!(loaded[1].number, 2);
    assert_eq!(loaded[1].severity, AlertSeverity::Low);
    assert_eq!(loaded[1].kind, AlertKind::CodeScanning);
    assert!(loaded[1].package.is_none());
    assert!(loaded[1].cvss_score.is_none());
}

#[tokio::test]
async fn get_security_alert_found() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};

    let db = Database::open_in_memory().await.unwrap();
    let alert = SecurityAlert {
        number: 7,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-9999".to_string(),
        package: Some("openssl".to_string()),
        vulnerable_range: Some("< 3.0".to_string()),
        fixed_version: Some("3.0.0".to_string()),
        cvss_score: Some(8.1),
        url: "https://github.com/acme/api/security/dependabot/7".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: "Buffer overflow in openssl".to_string(),
    };
    db.save_security_alerts(&[alert]).await.unwrap();

    let found = db
        .get_security_alert("acme/api", 7, AlertKind::Dependabot)
        .await
        .unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.number, 7);
    assert_eq!(found.title, "CVE-2024-9999");
    assert_eq!(found.package.as_deref(), Some("openssl"));
    assert_eq!(found.fixed_version.as_deref(), Some("3.0.0"));
}

#[tokio::test]
async fn get_security_alert_wrong_kind_returns_none() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};

    let db = Database::open_in_memory().await.unwrap();
    let alert = SecurityAlert {
        number: 7,
        repo: "acme/api".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-9999".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://github.com/acme/api/security/dependabot/7".to_string(),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: String::new(),
    };
    db.save_security_alerts(&[alert]).await.unwrap();

    // Same number, wrong kind
    let result = db
        .get_security_alert("acme/api", 7, AlertKind::CodeScanning)
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn get_security_alert_not_found() {
    use crate::models::AlertKind;
    let db = Database::open_in_memory().await.unwrap();
    let result = db
        .get_security_alert("acme/api", 999, AlertKind::Dependabot)
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn security_alerts_save_replaces_previous() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};

    let db = in_memory_db().await;
    let now = chrono::Utc::now();

    let alerts1 = vec![SecurityAlert {
        number: 1,
        repo: "acme/app".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "Old alert".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com/1".to_string(),
        created_at: now,
        state: "open".to_string(),
        description: "".to_string(),
    }];
    db.save_security_alerts(&alerts1).await.unwrap();
    assert_eq!(db.load_security_alerts().await.unwrap().len(), 1);

    let alerts2 = vec![SecurityAlert {
        number: 10,
        repo: "acme/new".to_string(),
        severity: AlertSeverity::Medium,
        kind: AlertKind::CodeScanning,
        title: "New alert".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com/10".to_string(),
        created_at: now,
        state: "open".to_string(),
        description: "".to_string(),
    }];
    db.save_security_alerts(&alerts2).await.unwrap();
    let loaded = db.load_security_alerts().await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].title, "New alert");
}

#[tokio::test]
async fn save_security_alerts_preserves_agent_fields() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};
    use chrono::Utc;

    let db = Database::open_in_memory().await.unwrap();

    let alert = SecurityAlert {
        number: 1,
        repo: "acme/app".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-1234".to_string(),
        package: Some("lodash".to_string()),
        vulnerable_range: None,
        fixed_version: Some("4.17.21".to_string()),
        cvss_score: Some(7.5),
        url: "https://github.com/acme/app/security/dependabot/1".to_string(),
        created_at: Utc::now(),
        state: "open".to_string(),
        description: "Prototype pollution".to_string(),
    };
    db.save_security_alerts(&[alert]).await.unwrap();

    // Simulate agent dispatch via the proper set_alert_agent method
    db.set_alert_agent(
        "acme/app",
        1,
        AlertKind::Dependabot,
        "dispatch:fix-1",
        "/tmp/wt",
    )
    .await
    .unwrap();

    // Refresh with updated alert data
    let refreshed = SecurityAlert {
        number: 1,
        repo: "acme/app".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-1234 (updated)".to_string(),
        package: Some("lodash".to_string()),
        vulnerable_range: None,
        fixed_version: Some("4.17.22".to_string()),
        cvss_score: Some(7.5),
        url: "https://github.com/acme/app/security/dependabot/1".to_string(),
        created_at: Utc::now(),
        state: "open".to_string(),
        description: "Prototype pollution".to_string(),
    };
    db.save_security_alerts(&[refreshed]).await.unwrap();

    let loaded = db.load_security_alerts().await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].title, "CVE-2024-1234 (updated)");
    assert_eq!(loaded[0].fixed_version.as_deref(), Some("4.17.22"));

    // Agent status should still be present after refresh
    let status = db
        .alert_agent_status("acme/app", 1, AlertKind::Dependabot)
        .await
        .unwrap();
    assert!(status.is_some(), "agent status should be preserved");
}

#[tokio::test]
async fn set_alert_agent_updates_fields() {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};
    use chrono::Utc;

    let db = Database::open_in_memory().await.unwrap();

    let alert = SecurityAlert {
        number: 1,
        repo: "acme/app".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com".to_string(),
        created_at: Utc::now(),
        state: "open".to_string(),
        description: String::new(),
    };
    db.save_security_alerts(&[alert]).await.unwrap();

    db.set_alert_agent(
        "acme/app",
        1,
        AlertKind::Dependabot,
        "dispatch:fix-1",
        "/tmp/wt",
    )
    .await
    .unwrap();

    let status = db
        .alert_agent_status("acme/app", 1, AlertKind::Dependabot)
        .await
        .unwrap();
    assert_eq!(
        status,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        "agent should be marked as reviewing"
    );
}

#[tokio::test]
async fn update_agent_status_finds_security_alert() {
    use crate::models::{AlertKind, AlertSeverity, ReviewAgentStatus, SecurityAlert};
    use chrono::Utc;

    let db = Database::open_in_memory().await.unwrap();
    let alert = SecurityAlert {
        number: 1,
        repo: "acme/app".to_string(),
        severity: AlertSeverity::High,
        kind: AlertKind::Dependabot,
        title: "CVE".to_string(),
        package: None,
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: "https://example.com".to_string(),
        created_at: Utc::now(),
        state: "open".to_string(),
        description: String::new(),
    };
    db.save_security_alerts(&[alert]).await.unwrap();
    db.set_alert_agent(
        "acme/app",
        1,
        AlertKind::Dependabot,
        "dispatch:fix-1",
        "/tmp/wt",
    )
    .await
    .unwrap();

    let table = db
        .update_agent_status("acme/app", 1, Some("findings_ready"))
        .await
        .unwrap();
    assert_eq!(table, "security_alerts");

    let status = db
        .alert_agent_status("acme/app", 1, AlertKind::Dependabot)
        .await
        .unwrap();
    assert_eq!(status, Some(ReviewAgentStatus::FindingsReady));
}
