//! Test resource subscription functionality.

use rmcp::transport::{child_process::TokioChildProcess, ConfigureCommandExt};
use rmcp::ServiceExt;
use tokio::process::Command;

fn build_stdio_server() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let output = std::process::Command::new("cargo")
            .args(["build", "--bin", "serial-mcp-server"])
            .output()
            .expect("cargo build");
        if !output.status.success() {
            panic!(
                "cargo build --bin serial-mcp-server failed:\nstderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    });
}

#[tokio::test]
async fn resource_subscription_works() {
    build_stdio_server();

    let cmd = Command::new(
        std::env::current_dir()
            .unwrap()
            .join("target/debug/serial-mcp-server"),
    )
    .configure(|cmd| {
        cmd.env("RUST_LOG", "warn");
    });

    let transport = TokioChildProcess::new(cmd).expect("spawn stdio server");
    let client = ().serve(transport).await.expect("initialize client");

    // Subscribe to a resource
    client
        .subscribe(rmcp::model::SubscribeRequestParams::new("serial://ports"))
        .await
        .expect("subscribe to ports resource");

    // Unsubscribe
    client
        .unsubscribe(rmcp::model::UnsubscribeRequestParams::new("serial://ports"))
        .await
        .expect("unsubscribe from ports resource");

    client.cancel().await.ok();
}

#[tokio::test]
async fn resource_subscribe_unsubscribe_roundtrip() {
    build_stdio_server();

    let cmd = Command::new(
        std::env::current_dir()
            .unwrap()
            .join("target/debug/serial-mcp-server"),
    )
    .configure(|cmd| {
        cmd.env("RUST_LOG", "warn");
    });

    let transport = TokioChildProcess::new(cmd).expect("spawn stdio server");
    let client = ().serve(transport).await.expect("initialize client");

    // Subscribe
    client
        .subscribe(rmcp::model::SubscribeRequestParams::new(
            "serial://connections",
        ))
        .await
        .expect("subscribe");

    // Subscribe again (should succeed - idempotent)
    client
        .subscribe(rmcp::model::SubscribeRequestParams::new(
            "serial://connections",
        ))
        .await
        .expect("subscribe again");

    // Unsubscribe
    client
        .unsubscribe(rmcp::model::UnsubscribeRequestParams::new(
            "serial://connections",
        ))
        .await
        .expect("unsubscribe");

    client.cancel().await.ok();
}
