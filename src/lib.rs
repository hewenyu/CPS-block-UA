mod policy;

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::host::{LogLevel, LogRequest},
    client::{AuthorError, ComposedPlugin, PluginBuilder},
};
use serde_json::Value;

pub use policy::{ConfigError, DEFAULT_PATTERN, Mode, Policy};
pub const MANIFEST: &[u8] = include_bytes!("../plugin.json");

pub fn build_plugin(policy: Policy) -> Result<ComposedPlugin, AuthorError> {
    let policy = Arc::new(policy);
    PluginBuilder::from_json(MANIFEST)?
        .middleware(move |call| {
            let policy = Arc::clone(&policy);
            async move {
                if let Some(reason) =
                    policy.rejection_reason(&call.request.head.endpoint, &call.request.head.headers)
                {
                    let enforce = policy.mode() == Mode::Enforce;
                    let log = LogRequest {
                        event: "ua_policy_decision".to_owned(),
                        level: LogLevel::Warn,
                        fields: BTreeMap::from([
                            ("reason".to_owned(), Value::from(reason)),
                            ("mode".to_owned(), Value::from(policy.mode().as_str())),
                            (
                                "decision".to_owned(),
                                Value::from(if enforce { "reject" } else { "would_reject" }),
                            ),
                        ]),
                    };
                    if let Ok(params) = serde_json::to_value(log) {
                        // Logging must not turn a policy denial into a host timeout/fail-open.
                        let _ = tokio::time::timeout(
                            Duration::from_millis(100),
                            call.host.call("host.log", params, Vec::new()),
                        )
                        .await;
                    }
                    if enforce {
                        return Err(PluginFault::new(
                            ErrorCode::Rejected,
                            "Client User-Agent is not allowed",
                        ));
                    }
                }
                call.next.run(call.request).await
            }
        })?
        .build()
}
