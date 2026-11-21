//! How hard the computer is working, read from /proc and /sys every couple of seconds.

use super::Out;
use crate::model::{Machine, Update};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default)]
struct Times {
    idle: u64,
    total: u64,
}

fn times(line: &str) -> Times {
    let values: Vec<u64> = line.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
    let idle = values.get(3).copied().unwrap_or(0) + values.get(4).copied().unwrap_or(0);
    Times { idle, total: values.iter().sum() }
}

fn load(now: Times, before: Option<Times>) -> Option<f32> {
    let before = before?;
    (now.total > before.total)
        .then(|| 1.0 - (now.idle.saturating_sub(before.idle)) as f32 / (now.total - before.total) as f32)
}

/// The processor package if there is one, else anything that sounds like a CPU.
fn temperature() -> f32 {
    let mut zones = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/thermal") {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("thermal_zone")) {
                continue;
            }
            let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default().trim().to_string();
            let Some(milli) = std::fs::read_to_string(path.join("temp")).ok().and_then(|t| t.trim().parse::<f32>().ok()) else {
                continue;
            };
            zones.push((kind, milli / 1000.0));
        }
    }
    // hwmon has the AMD and most other CPU sensors
    if let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = std::fs::read_to_string(path.join("name")).unwrap_or_default().trim().to_string();
            if let Some(milli) = std::fs::read_to_string(path.join("temp1_input")).ok().and_then(|t| t.trim().parse::<f32>().ok()) {
                zones.push((name, milli / 1000.0));
            }
        }
    }
    let find = |test: &dyn Fn(&str) -> bool| zones.iter().find(|(kind, _)| test(kind)).map(|(_, t)| *t);
    find(&|k| k == "x86_pkg_temp")
        .or_else(|| find(&|k| k == "k10temp" || k == "coretemp" || k == "zenpower"))
        .or_else(|| find(&|k| {
            let k = k.to_lowercase();
            k.contains("cpu") || k.contains("pkg") || k.contains("soc")
        }))
        .unwrap_or(0.0)
}

/// How busy the graphics card is (amdgpu says; others mostly don't).
fn gpu_busy() -> Option<f32> {
    let cards = std::fs::read_dir("/sys/class/drm").ok()?;
    cards
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("card") && !e.file_name().to_string_lossy().contains('-'))
        .find_map(|e| std::fs::read_to_string(e.path().join("device/gpu_busy_percent")).ok())
        .and_then(|t| t.trim().parse::<f32>().ok())
        .map(|p| (p / 100.0).clamp(0.0, 1.0))
}

/// Real filesystems, one per device: where they're mounted, used and total bytes.
fn disks() -> Vec<(String, u64, u64)> {
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (Some(device), Some(point), Some(kind)) = (fields.next(), fields.next(), fields.next()) else { continue };
        if !device.starts_with("/dev/") || !["ext4", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs3", "zfs", "bcachefs"].contains(&kind) || seen.contains(&device.to_string()) {
            continue;
        }
        seen.push(device.to_string());
        let point = point.replace("\\040", " ");
        if let Ok(stat) = rustix::fs::statvfs(point.as_str()) {
            let total = stat.f_blocks * stat.f_frsize;
            let free = stat.f_bavail * stat.f_frsize;
            if total > 0 {
                out.push((point, total.saturating_sub(free), total));
            }
        }
    }
    out.truncate(4);
    out
}

/// Every process: its name, CPU time (in clock ticks) so far, memory, and owner.
fn processes() -> Vec<(u32, String, u64, u64, u32)> {
    let mut list = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return list };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
        // The name sits in parentheses and may hold spaces or parentheses itself
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else { continue };
        let mut name = stat[open + 1..close].to_string();
        // The kernel keeps 15 characters of a name: the command line has the rest
        if name.len() == 15
            && let Ok(cmdline) = std::fs::read(entry.path().join("cmdline"))
            && let Some(first) = cmdline.split(|b| *b == 0).next()
        {
            let full = String::from_utf8_lossy(first).rsplit('/').next().unwrap_or("").to_string();
            if full.starts_with(&name) {
                name = full;
            }
        }
        let rest: Vec<&str> = stat[close + 1..].split_whitespace().collect();
        let ticks = rest.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + rest.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        let rss_pages = rest.get(21).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        let uid = std::fs::metadata(entry.path()).map(|m| std::os::unix::fs::MetadataExt::uid(&m)).unwrap_or(0);
        list.push((pid, name, ticks, rss_pages * 4, uid));
    }
    list
}

/// Adds a sample, keeping the last 60.
fn sample<T>(list: &mut Vec<T>, value: T) {
    list.push(value);
    if list.len() > 60 {
        list.remove(0);
    }
}

pub async fn run(out: Out, mut commands: tokio::sync::mpsc::UnboundedReceiver<crate::model::Command>) {
    let mut watching = false;
    let mut last_ticks: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    let mut last_total: Option<u64> = None;
    let me = rustix::process::getuid().as_raw();
    let mut last: Option<Times> = None;
    let mut last_cores: Vec<Times> = Vec::new();
    let mut last_net: Option<(u64, u64, Instant)> = None;
    let mut start_net: Option<(u64, u64)> = None;
    let mut machine = Machine::default();
    let mut ticker = tokio::time::interval(Duration::from_secs(2));
    let mut round = 0u32;

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            command = commands.recv() => {
                match command {
                    Some(crate::model::Command::WatchProcesses(on)) => {
                        watching = on;
                        last_ticks.clear();
                        last_total = None;
                        if !on {
                            out.send(Update::Processes(Vec::new()));
                        }
                    }
                    Some(crate::model::Command::EndProcess(pid)) => {
                        if let Some(pid) = rustix::process::Pid::from_raw(pid as i32)
                            && let Err(error) = rustix::process::kill_process(pid, rustix::process::Signal::TERM)
                        {
                            log::info!("couldn't end {pid:?}: {error}");
                        }
                    }
                    Some(_) => {}
                    None => return,
                }
                continue;
            }
        }
        let stat = tokio::fs::read_to_string("/proc/stat").await.unwrap_or_default();
        let total_ticks: u64 = stat.lines().next().map(|l| l.split_whitespace().skip(1).filter_map(|v| v.parse::<u64>().ok()).sum()).unwrap_or(0);
        let mut cpu_lines = stat.lines().filter(|l| l.starts_with("cpu"));
        if let Some(all) = cpu_lines.next().map(times) {
            machine.cpu = load(all, last).unwrap_or(machine.cpu);
            last = Some(all);
        }
        let cores: Vec<Times> = cpu_lines.map(times).collect();
        if cores.len() == last_cores.len() {
            machine.cores = cores.iter().zip(&last_cores).map(|(now, before)| load(*now, Some(*before)).unwrap_or(0.0)).collect();
        }
        last_cores = cores;

        let meminfo = tokio::fs::read_to_string("/proc/meminfo").await.unwrap_or_default();
        let field = |name: &str| {
            meminfo
                .lines()
                .find(|l| l.starts_with(name))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
        };
        let (total, available) = (field("MemTotal:"), field("MemAvailable:"));
        if total > 0 {
            machine.memory_total_kib = total;
            machine.memory_used_kib = total.saturating_sub(available);
            machine.memory = machine.memory_used_kib as f32 / total as f32;
        }

        // Every interface but the loopback, added up
        let netdev = tokio::fs::read_to_string("/proc/net/dev").await.unwrap_or_default();
        let (mut received, mut sent) = (0u64, 0u64);
        for line in netdev.lines().skip(2) {
            let Some((name, counters)) = line.split_once(':') else { continue };
            if name.trim() == "lo" {
                continue;
            }
            let fields: Vec<u64> = counters.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            received += fields.first().copied().unwrap_or(0);
            sent += fields.get(8).copied().unwrap_or(0);
        }
        let now = Instant::now();
        if let Some((r, s, at)) = last_net {
            let seconds = now.duration_since(at).as_secs_f64().max(0.001);
            machine.download = received.saturating_sub(r) as f64 / seconds;
            machine.upload = sent.saturating_sub(s) as f64 / seconds;
        }
        last_net = Some((received, sent, now));
        let (r0, s0) = *start_net.get_or_insert((received, sent));
        machine.downloaded = received.saturating_sub(r0);
        machine.uploaded = sent.saturating_sub(s0);

        // Sensors are slower to read, every other round is plenty
        if round.is_multiple_of(2) {
            machine.temperature = tokio::task::spawn_blocking(temperature).await.unwrap_or(0.0);
        }
        round = round.wrapping_add(1);
        machine.gpu = gpu_busy();
        if round % 15 == 1 {
            machine.disks = tokio::task::spawn_blocking(disks).await.unwrap_or_default();
        }
        let proc_text = |path: &'static str| async move { tokio::fs::read_to_string(path).await.unwrap_or_default() };
        machine.uptime = proc_text("/proc/uptime").await.split('.').next().and_then(|v| v.parse().ok()).unwrap_or(0);
        machine.load = proc_text("/proc/loadavg").await.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let history = &mut machine.history;
        sample(&mut history.cpu, machine.cpu);
        sample(&mut history.memory, machine.memory);
        sample(&mut history.gpu, machine.gpu.unwrap_or(0.0));
        sample(&mut history.download, machine.download);
        sample(&mut history.upload, machine.upload);
        out.send(Update::Machine(machine.clone()));

        // What's running, only while someone's looking: CPU from the change in ticks
        if watching {
            let now = tokio::task::spawn_blocking(processes).await.unwrap_or_default();
            let elapsed = last_total.map(|t| total_ticks.saturating_sub(t)).unwrap_or(0);
            let mut list: Vec<crate::model::Process> = now
                .iter()
                .map(|(pid, name, ticks, memory, uid)| {
                    let before = last_ticks.get(pid).copied().unwrap_or(*ticks);
                    let cpu = if elapsed > 0 { ticks.saturating_sub(before) as f32 / elapsed as f32 } else { 0.0 };
                    crate::model::Process { pid: *pid, name: name.clone(), cpu, memory_kib: *memory, own: *uid == me }
                })
                .collect();
            list.retain(|p| p.memory_kib > 0);
            last_ticks = now.iter().map(|(pid, _, ticks, ..)| (*pid, *ticks)).collect();
            last_total = Some(total_ticks);
            out.send(Update::Processes(list));
        }
    }
}
