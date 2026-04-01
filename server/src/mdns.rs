use mdns_sd::{ServiceDaemon, ServiceInfo};

const SERVICE_TYPE: &str = "_cast-media._tcp.local.";

pub async fn advertise(name: &str, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let mdns = ServiceDaemon::new()?;

    let host = hostname::get().unwrap_or_default().to_string_lossy().to_string();
    let host_label = format!("{host}.local.");

    let service = ServiceInfo::new(SERVICE_TYPE, name, &host_label, "", port, [("version", "1")].as_slice())?;

    mdns.register(service)?;

    tracing::info!("mDNS: advertising as '{}' on port {}", name, port);

    // Keep running forever — the ServiceDaemon handles responses in background
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}
