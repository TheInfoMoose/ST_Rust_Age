#[cfg(target_os = "linux")]
use std::process::Command;

pub struct Firewall;

impl Firewall {
    /// Opens a TCP port on Linux firewalls using pkexec.
    /// On Windows and MacOS, the OS automatically prompts the user when the app binds.
    pub fn open_port(port: u16) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            // Try firewalld first
            if Command::new("which")
                .arg("firewall-cmd")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                let status = Command::new("pkexec")
                    .args(["firewall-cmd", "--add-port", &format!("{}/tcp", port)])
                    .status()
                    .map_err(|e| format!("Failed to run pkexec firewall-cmd: {}", e))?;
                if status.success() {
                    return Ok(());
                }
            }
            // Try ufw
            if Command::new("which")
                .arg("ufw")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                let status = Command::new("pkexec")
                    .args(["ufw", "allow", &format!("{}/tcp", port)])
                    .status()
                    .map_err(|e| format!("Failed to run pkexec ufw: {}", e))?;
                if status.success() {
                    return Ok(());
                }
            }
            Err(
                "Failed to open port. Neither firewalld nor ufw could be successfully modified."
                    .into(),
            )
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = port;
            Ok(())
        }
    }

    /// Closes a TCP port on Linux firewalls.
    pub fn close_port(port: u16) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            if Command::new("which")
                .arg("firewall-cmd")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                let _ = Command::new("pkexec")
                    .args(["firewall-cmd", "--remove-port", &format!("{}/tcp", port)])
                    .status();
                return Ok(());
            }
            if Command::new("which")
                .arg("ufw")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                let _ = Command::new("pkexec")
                    .args(["ufw", "delete", "allow", &format!("{}/tcp", port)])
                    .status();
                return Ok(());
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = port;
            Ok(())
        }
    }
}

/// Represents an active scoped WireGuard WAN tunnel managed by `VpnTunnelManager`.
#[derive(Debug, Clone)]
pub struct VpnTunnelHandle {
    pub connection_id: uuid::Uuid,
    pub interface_name: String,
    pub endpoint: String,
    pub scoped_route: String,
    pub persistent: bool,
    pub temp_conf_path: Option<std::path::PathBuf>,
}

/// Manages the lifecycle of optional WireGuard WAN tunnels per `ConnectionProfile`.
/// Brings up a scoped `/32` route tunnel before SSH handshake when `vpn_config` is enabled,
/// and tears it down cleanly when jobs complete (unless marked `persistent_tunnel` for frequent schedules).
#[derive(Clone, Default)]
pub struct VpnTunnelManager {
    active_tunnels:
        std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<uuid::Uuid, VpnTunnelHandle>>>,
}

impl VpnTunnelManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Computes a `/32` (IPv4) or `/128` (IPv6) scoped route for the target SSH host
    /// so the VPN tunnel routes only traffic destined for the target server.
    pub fn compute_scoped_route(target_ssh_host: &str) -> String {
        let host_part = target_ssh_host
            .rsplit('@')
            .next()
            .unwrap_or(target_ssh_host)
            .trim();
        let ip_only = if let Some((h, _port)) = host_part.rsplit_once(':')
            && !h.contains(':')
        {
            h
        } else {
            host_part
        };

        if ip_only.contains('/') {
            ip_only.to_string()
        } else if ip_only.contains(':') {
            format!("{}/128", ip_only)
        } else {
            format!("{}/32", ip_only)
        }
    }

    /// Renders a WireGuard configuration string with `AllowedIPs` scoped to the target SSH host `/32`.
    pub fn render_scoped_wireguard_conf(
        conf: &simply_transfer_crypto::WireGuardConf,
        target_ssh_host: &str,
    ) -> String {
        let scoped_route = Self::compute_scoped_route(target_ssh_host);
        let mut allowed = vec![scoped_route];
        for ip in &conf.allowed_ips {
            let trimmed = ip.trim();
            if !trimmed.is_empty()
                && trimmed != "0.0.0.0/0"
                && trimmed != "::/0"
                && !allowed.iter().any(|a| a == trimmed)
            {
                allowed.push(trimmed.to_string());
            }
        }

        let mut out = String::new();
        out.push_str("[Interface]\n");
        out.push_str(&format!("PrivateKey = {}\n", conf.private_key));
        out.push_str(&format!("Address = {}\n", conf.address));
        if let Some(ref dns) = conf.dns {
            out.push_str(&format!("DNS = {}\n", dns));
        }
        if let Some(mtu) = conf.mtu {
            out.push_str(&format!("MTU = {}\n", mtu));
        }
        out.push_str("\n[Peer]\n");
        out.push_str(&format!("PublicKey = {}\n", conf.public_key));
        if let Some(ref psk) = conf.preshared_key {
            out.push_str(&format!("PresharedKey = {}\n", psk));
        }
        out.push_str(&format!("Endpoint = {}\n", conf.endpoint));
        out.push_str(&format!("AllowedIPs = {}\n", allowed.join(", ")));
        if let Some(keepalive) = conf.persistent_keepalive {
            out.push_str(&format!("PersistentKeepalive = {}\n", keepalive));
        }
        out
    }

    /// Brings up the WireGuard tunnel for `connection_id` before initiating the TCP/SSH handshake.
    pub async fn bring_up_tunnel(
        &self,
        connection_id: uuid::Uuid,
        vpn_config: &simply_transfer_crypto::VpnConfig,
        storage: &simply_transfer_crypto::SecureStorage,
        target_ssh_host: &str,
    ) -> Result<VpnTunnelHandle, String> {
        if !vpn_config.enabled {
            return Err("VPN configuration is not enabled".to_string());
        }

        {
            let read_guard = self.active_tunnels.read().await;
            if let Some(existing) = read_guard.get(&connection_id) {
                return Ok(existing.clone());
            }
        }

        let (priv_key, psk) = vpn_config
            .decrypt_keys(storage)
            .map_err(|e| format!("Failed to decrypt VPN keys: {}", e))?;

        let decrypted = simply_transfer_crypto::WireGuardConf {
            private_key: priv_key,
            address: vpn_config.address.clone(),
            dns: vpn_config.dns.clone(),
            mtu: vpn_config.mtu,
            public_key: vpn_config.peer_public_key.clone(),
            preshared_key: psk,
            endpoint: vpn_config.endpoint.clone(),
            allowed_ips: vpn_config.allowed_ips.clone(),
            persistent_keepalive: vpn_config.persistent_keepalive,
        };

        let scoped_route = Self::compute_scoped_route(target_ssh_host);
        let rendered_conf = Self::render_scoped_wireguard_conf(&decrypted, target_ssh_host);
        let short_id: String = connection_id.simple().to_string().chars().take(8).collect();
        let interface_name = format!("stwg{}", short_id);

        let temp_conf_path = std::env::temp_dir().join(format!("{}.conf", interface_name));
        let _ = std::fs::write(&temp_conf_path, &rendered_conf);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                std::fs::set_permissions(&temp_conf_path, std::fs::Permissions::from_mode(0o600));
        }

        // Best-effort invocation of wg-quick if present on the system
        #[cfg(target_os = "linux")]
        {
            if Command::new("which")
                .arg("wg-quick")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                let _ = Command::new("wg-quick")
                    .args(["up", temp_conf_path.to_str().unwrap_or("")])
                    .output();
            }
        }

        let handle = VpnTunnelHandle {
            connection_id,
            interface_name,
            endpoint: vpn_config.endpoint.clone(),
            scoped_route,
            persistent: vpn_config.persistent_tunnel,
            temp_conf_path: Some(temp_conf_path),
        };

        self.active_tunnels
            .write()
            .await
            .insert(connection_id, handle.clone());
        Ok(handle)
    }

    /// Tears down the WireGuard tunnel for `connection_id` when a job finishes,
    /// unless `persistent` is true and `force` is false.
    /// Returns `Ok(true)` if the tunnel was torn down, or `Ok(false)` if kept alive/not active.
    pub async fn teardown_tunnel(
        &self,
        connection_id: uuid::Uuid,
        force: bool,
    ) -> Result<bool, String> {
        let mut write_guard = self.active_tunnels.write().await;
        if let Some(existing) = write_guard.get(&connection_id)
            && existing.persistent
            && !force
        {
            tracing::info!(
                "Keeping persistent WAN VPN tunnel {} alive for scheduled jobs",
                existing.interface_name
            );
            return Ok(false);
        }

        if let Some(handle) = write_guard.remove(&connection_id) {
            if let Some(ref conf_path) = handle.temp_conf_path {
                #[cfg(target_os = "linux")]
                {
                    if Command::new("which")
                        .arg("wg-quick")
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false)
                    {
                        let _ = Command::new("wg-quick")
                            .args(["down", conf_path.to_str().unwrap_or("")])
                            .output();
                    }
                }
                let _ = std::fs::remove_file(conf_path);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn is_tunnel_active(&self, connection_id: uuid::Uuid) -> bool {
        self.active_tunnels
            .read()
            .await
            .contains_key(&connection_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simply_transfer_crypto::{SecureStorage, VpnConfig, WireGuardConf};

    #[tokio::test]
    async fn test_vpn_tunnel_manager_scoped_route_and_lifecycle() {
        assert_eq!(
            VpnTunnelManager::compute_scoped_route("backup@198.51.100.42:2222"),
            "198.51.100.42/32"
        );

        let raw = r#"
[Interface]
PrivateKey = cHJpdmF0ZWtleXJlYWxseWxvbmdiYXNlNjRzdHJpbmc=
Address = 10.88.0.2/24

[Peer]
PublicKey = cHVibGlja2V5cmVhbGx5bG9uZ2Jhc2U2NHN0cmluZw==
Endpoint = vpn.example.com:51820
AllowedIPs = 0.0.0.0/0
"#;
        let parsed = WireGuardConf::parse(raw).unwrap();
        let rendered = VpnTunnelManager::render_scoped_wireguard_conf(&parsed, "10.88.0.1");
        assert!(rendered.contains("AllowedIPs = 10.88.0.1/32"));
        assert!(!rendered.contains("0.0.0.0/0"));

        let storage = SecureStorage::new("simply_transfer_vpn_mgr_test");
        let mut vpn_cfg = VpnConfig::from_wireguard_conf(&parsed, &storage).unwrap();
        let mgr = VpnTunnelManager::new();
        let conn_id = uuid::Uuid::new_v4();

        // Non-persistent tunnel tears down on job completion
        let handle = mgr
            .bring_up_tunnel(conn_id, &vpn_cfg, &storage, "10.88.0.1")
            .await
            .unwrap();
        assert_eq!(handle.scoped_route, "10.88.0.1/32");
        assert!(mgr.is_tunnel_active(conn_id).await);
        assert!(mgr.teardown_tunnel(conn_id, false).await.unwrap());
        assert!(!mgr.is_tunnel_active(conn_id).await);

        // Persistent tunnel stays active on non-forced teardown, tears down when forced
        vpn_cfg.persistent_tunnel = true;
        mgr.bring_up_tunnel(conn_id, &vpn_cfg, &storage, "10.88.0.1")
            .await
            .unwrap();
        assert!(!mgr.teardown_tunnel(conn_id, false).await.unwrap());
        assert!(mgr.is_tunnel_active(conn_id).await);
        assert!(mgr.teardown_tunnel(conn_id, true).await.unwrap());
        assert!(!mgr.is_tunnel_active(conn_id).await);
    }
}
