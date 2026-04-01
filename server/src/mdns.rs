const SERVICE_TYPE: &str = "_cast-media._tcp";

pub async fn advertise(name: &str, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    // Use the system's dns-sd command on macOS for reliable Bonjour registration.
    // On other platforms, fall back to the mdns-sd Rust library.
    if cfg!(target_os = "macos") || cfg!(target_os = "linux") {
        advertise_native(name, port).await
    } else {
        advertise_mdns_sd(name, port).await
    }
}

/// Register via the system dns-sd command (macOS) or avahi-publish (Linux).
/// This is more reliable than the Rust mdns-sd library as it uses the OS mDNS responder.
async fn advertise_native(name: &str, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let (cmd, args) = if cfg!(target_os = "macos") {
        ("dns-sd", vec![
            "-R".to_string(),
            name.to_string(),
            SERVICE_TYPE.to_string(),
            "local".to_string(),
            port.to_string(),
        ])
    } else {
        // Linux: avahi-publish
        ("avahi-publish-service", vec![
            name.to_string(),
            format!("{SERVICE_TYPE}."),
            port.to_string(),
        ])
    };

    let mut child = tokio::process::Command::new(cmd)
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            tracing::warn!("Could not start {cmd} for Bonjour: {e}");
            e
        })?;

    tracing::info!("mDNS: advertising as '{name}' on port {port} (via {cmd})");

    // Keep running — the child process handles mDNS responses
    let _ = child.wait().await;
    Ok(())
}

/// Fallback: use the mdns-sd Rust library (Windows, or if native command unavailable)
async fn advertise_mdns_sd(name: &str, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    use mdns_sd::{ServiceDaemon, ServiceInfo};

    let mdns = ServiceDaemon::new()?;
    let host = hostname::get().unwrap_or_default().to_string_lossy().to_string();
    let host_label = format!("{host}.local.");

    let service = ServiceInfo::new(
        &format!("{SERVICE_TYPE}.local."),
        name,
        &host_label,
        "",
        port,
        [("version", "1")].as_slice(),
    )?;

    mdns.register(service)?;
    tracing::info!("mDNS: advertising as '{name}' on port {port} (via mdns-sd library)");

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}
