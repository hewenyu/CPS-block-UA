use std::time::Duration;

use cps_block_ua::{Policy, build_plugin};
use gateway_plugin_sdk::client::{PluginSession, SessionConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = PluginSession::accept(
        tokio::io::stdin(),
        tokio::io::stdout(),
        SessionConfig {
            // A timed-out log callback stays pending in the SDK until the parent ends.
            // Reserve both log and next slots for every admitted call.
            maximum_calls: 32,
            maximum_callbacks: 64,
            // Accept host-configured long requests. The host's per-call deadline still applies;
            // passing the opaque response handle does not buffer the response stream here.
            maximum_call_timeout: Duration::from_secs(24 * 60 * 60),
            ..SessionConfig::default()
        },
    )
    .await?;
    let policy = Policy::from_value(session.handshake().configuration.clone())?;
    session.run(build_plugin(policy)?).await?;
    Ok(())
}
