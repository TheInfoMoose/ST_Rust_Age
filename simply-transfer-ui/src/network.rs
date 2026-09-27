use std::process::Command;

pub fn get_active_network_info() -> (String, String) {
    let mut link_type = "Unknown".to_string();
    let mut max_throughput = "Variable".to_string();

    #[cfg(target_os = "linux")]
    {
        // Simple heuristic for Linux using `ip route` to find default interface
        if let Ok(output) = Command::new("sh")
            .arg("-c")
            .arg("ip route | grep default | awk '{print $5}'")
            .output()
        {
            let iface = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !iface.is_empty() {
                if iface.starts_with("wl") || iface.starts_with("wlan") {
                    // It's Wi-Fi
                    if let Ok(iw_out) = Command::new("iw")
                        .arg("dev")
                        .arg(&iface)
                        .arg("link")
                        .output()
                    {
                        let iw_str = String::from_utf8_lossy(&iw_out.stdout);
                        if iw_str.contains("tx bitrate") {
                            let bitrate_line = iw_str
                                .lines()
                                .find(|l| l.contains("tx bitrate"))
                                .unwrap_or("");
                            let freq_line =
                                iw_str.lines().find(|l| l.contains("freq:")).unwrap_or("");

                            let mut freq_type = "Wi-Fi";
                            if freq_line.contains("24") {
                                freq_type = "2.4GHz Wi-Fi";
                            } else if freq_line.contains("5") {
                                freq_type = "5GHz Wi-Fi";
                            } else if freq_line.contains("6") {
                                freq_type = "6GHz Wi-Fi";
                            }

                            link_type = freq_type.to_string();
                            max_throughput = bitrate_line
                                .trim()
                                .replace("tx bitrate:", "")
                                .trim()
                                .to_string();
                        } else {
                            link_type = "Wi-Fi".to_string();
                        }
                    }
                } else {
                    // It's Ethernet
                    if let Ok(speed_out) =
                        std::fs::read_to_string(format!("/sys/class/net/{}/speed", iface))
                    {
                        if let Ok(speed_mbps) = speed_out.trim().parse::<u32>() {
                            match speed_mbps {
                                10..=100 => {
                                    link_type = "FE".to_string();
                                    max_throughput = "~12 MB/s".to_string();
                                }
                                101..=1000 => {
                                    link_type = "GbE".to_string();
                                    max_throughput = "~125 MB/s".to_string();
                                }
                                1001..=2500 => {
                                    link_type = "2.5 GbE".to_string();
                                    max_throughput = "~312 MB/s".to_string();
                                }
                                2501..=5000 => {
                                    link_type = "5 GbE".to_string();
                                    max_throughput = "~625 MB/s".to_string();
                                }
                                5001..=10000 => {
                                    link_type = "10 GbE".to_string();
                                    max_throughput = "~1250 MB/s".to_string();
                                }
                                _ => {
                                    link_type = format!("{} Mbps Ethernet", speed_mbps);
                                    max_throughput = format!("~{} MB/s", speed_mbps / 8);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        // Use PowerShell to get active NetAdapter info
        let ps_cmd = "Get-NetAdapter | Where-Object { $_.Status -eq 'Up' } | Select-Object Name, MediaType, LinkSpeed | ConvertTo-Json";
        if let Ok(output) = Command::new("powershell")
            .arg("-Command")
            .arg(ps_cmd)
            .output()
        {
            let json_str = String::from_utf8_lossy(&output.stdout);
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
                // Handle both single object and array
                let adapter = if parsed.is_array() {
                    parsed.as_array().unwrap().first()
                } else {
                    Some(&parsed)
                };

                if let Some(adapter) = adapter {
                    let name = adapter["Name"].as_str().unwrap_or("").to_lowercase();
                    let speed_str = adapter["LinkSpeed"].as_str().unwrap_or("");

                    if name.contains("wi-fi") || name.contains("wireless") {
                        link_type = "Wi-Fi".to_string();
                        max_throughput = speed_str.to_string(); // e.g., "866.7 Mbps"
                    } else {
                        if speed_str.contains("100 Mbps") {
                            link_type = "FE".to_string();
                            max_throughput = "~12 MB/s".to_string();
                        } else if speed_str.contains("1 Gbps") {
                            link_type = "GbE".to_string();
                            max_throughput = "~125 MB/s".to_string();
                        } else if speed_str.contains("2.5 Gbps") {
                            link_type = "2.5 GbE".to_string();
                            max_throughput = "~312 MB/s".to_string();
                        } else if speed_str.contains("5 Gbps") {
                            link_type = "5 GbE".to_string();
                            max_throughput = "~625 MB/s".to_string();
                        } else if speed_str.contains("10 Gbps") {
                            link_type = "10 GbE".to_string();
                            max_throughput = "~1250 MB/s".to_string();
                        } else {
                            link_type = "Ethernet".to_string();
                            max_throughput = speed_str.to_string();
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("networksetup")
            .arg("-listallhardwareports")
            .output()
        {
            link_type = "Mac Network".to_string();
            max_throughput = "Variable".to_string();
        }
    }

    (link_type, max_throughput)
}
