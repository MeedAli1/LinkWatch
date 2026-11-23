use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairStepResult {
    pub step_name: String,
    pub command: String,
    pub success: bool,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRepairResult {
    pub steps: Vec<RepairStepResult>,
    pub overall_success: bool,
    pub summary: String,
    pub timestamp_ms: i64,
}

/// Aggregates per-step results into the overall verdict. Every step counts -
/// a failed one is never excepted. An empty run is not a success.
fn summarize_steps(steps: &[RepairStepResult]) -> (bool, String) {
    if !steps.is_empty() && steps.iter().all(|s| s.success) {
        return (
            true,
            "Network stack refreshed successfully. DNS cache cleared and local DHCP lease renewed.".to_string(),
        );
    }
    let failed: Vec<&str> = steps
        .iter()
        .filter(|s| !s.success)
        .map(|s| s.step_name.as_str())
        .collect();
    let detail = if failed.is_empty() {
        "no steps were executed".to_string()
    } else {
        format!("failed steps: {} - see each step's output above; clearing the ARP table requires an elevated (administrator) run", failed.join(", "))
    };
    (false, format!("Repair incomplete - {}.", detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(name: &str, ok: bool) -> RepairStepResult {
        RepairStepResult {
            step_name: name.to_string(),
            command: format!("test {}", name),
            success: ok,
            output: String::new(),
        }
    }

    #[test]
    fn all_steps_success_yields_overall_success() {
        let steps = vec![step("Flush DNS Cache", true), step("Clear Local Route & ARP Table", true)];
        let (ok, summary) = summarize_steps(&steps);
        assert!(ok);
        assert!(summary.contains("refreshed successfully"));
    }

    #[test]
    fn failed_arp_step_is_not_swallowed() {
        // The regression: arp exits non-zero unelevated with empty stdout,
        // previously excused as success; now it must fail the overall result.
        let steps = vec![
            step("Flush DNS Cache", true),
            step("Clear Local Route & ARP Table", false),
        ];
        let (ok, summary) = summarize_steps(&steps);
        assert!(!ok, "a refused arp step must fail the repair, not hide it");
        assert!(summary.contains("Clear Local Route & ARP Table"));
        assert!(summary.to_lowercase().contains("elevated"));
    }

    #[test]
    fn empty_run_is_not_success() {
        let (ok, _) = summarize_steps(&[]);
        assert!(!ok);
    }
}
/// Automates standard Windows network stack repair operations:
/// 1. Flushes the Windows DNS resolver cache (`ipconfig /flushdns`)
/// 2. Registers and refreshes DNS names (`ipconfig /registerdns`)
/// 3. Releases and renews the local DHCP lease (`ipconfig /renew`)
/// 4. Clears temporary routing and ARP cache entries
pub fn execute_network_repair() -> NetworkRepairResult {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let commands = [
            ("Flush DNS Cache", "ipconfig", vec!["/flushdns"]),
            ("Register DNS Names", "ipconfig", vec!["/registerdns"]),
            ("Renew DHCP Lease", "ipconfig", vec!["/renew"]),
            ("Clear Local Route & ARP Table", "arp", vec!["-d", "*"]),
        ];

        let mut steps = Vec::new();

        for (name, prog, args) in commands {
            let cmd_str = format!("{} {}", prog, args.join(" "));
            let mut cmd = Command::new(prog);
            cmd.args(&args).creation_flags(CREATE_NO_WINDOW);

            match cmd.output() {
                Ok(output) => {
                    let out_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let err_str = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    // Success is the exit status alone. arp -d * needs
                    // elevation; when the OS refuses it exits non-zero with
                    // empty stdout, and the old "empty stdout means ok"
                    // allowance turned that refusal into a reported "cleared".
                    let is_ok = output.status.success();

                    let combined = if !err_str.is_empty() {
                        format!("{}\n{}", out_str, err_str).trim().to_string()
                    } else if out_str.is_empty() {
                        "Operation completed successfully.".to_string()
                    } else {
                        out_str
                    };

                    steps.push(RepairStepResult {
                        step_name: name.to_string(),
                        command: cmd_str,
                        success: is_ok,
                        output: combined,
                    });
                }
                Err(e) => {
                    steps.push(RepairStepResult {
                        step_name: name.to_string(),
                        command: cmd_str,
                        success: false,
                        output: format!("Execution failed: {}", e),
                    });
                }
            }
        }

        let (all_succeeded, summary) = summarize_steps(&steps);

        NetworkRepairResult {
            steps,
            overall_success: all_succeeded,
            summary,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        NetworkRepairResult {
            steps: Vec::new(),
            overall_success: true,
            summary: "Network repair simulation completed (non-Windows platform).".to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
        }
    }
}
