//! Live check that the guest cannot reach the host's loopback interface.
//!
//! The emulator forwards `10.0.2.2` (IPv4) and `fec0::2` (IPv6) to the Windows
//! loopback interface, so a guest app with Android's Internet permission could
//! otherwise open the adb server's smart socket or any other local listener. This
//! test boots or reuses the emulator through the runtime, which installs the
//! egress rules, and then asks from *inside* Android whether a host listener is
//! reachable — an honest answer, not the presence of a rule:
//!
//! 1. a control connect from the host proves the listener really is listening;
//! 2. the same port is refused from the guest, over IPv4 and over IPv6;
//! 3. the adb smart socket is refused, not merely slow;
//! 4. DNS and the internet still work, so the fix did not break networking;
//! 5. removing the rule lets the guest's connection reach the listener again
//!    (the host accepts it, and the listener answers), which is what proves the
//!    refusal comes from this rule and not from something else.
//!
//! It needs a running emulator, so it is ignored by default:
//!
//! ```text
//! cargo test --test isolation_live -- --ignored --nocapture
//! ```

mod support;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use support::Harness;

use droidwin_lib::runtime::adb::Adb;

const V4_RULE: &str = "iptables -D OUTPUT -d 10.0.2.2 -j REJECT";
const V4_ADD: &str = "iptables -I OUTPUT 1 -d 10.0.2.2 -j REJECT";
const V4_MARKER: &str = "HOST-LISTENER";

/// What the guest saw when it tried to open a host port.
struct Probe {
    refused: bool,
    text: String,
    /// The probe itself failed (adb timed out, shell missing) — never read that
    /// as a blocked guest.
    error: Option<String>,
}

/// One connection attempt made from inside Android, the way a guest app would.
async fn probe(adb: &Adb, serial: &str, host: &str, port: u16) -> Probe {
    // `timeout` bounds the whole attempt. A listener that accepts and then stays
    // silent is the *reachable* case, and without this the adb shell would wait
    // on it until the adb call timed out — the failure this test found first.
    let line =
        format!("printf 'PING' | timeout 6 toybox nc -w 2 {host} {port} 2>&1; echo \"|done\"");
    match adb
        .run(
            &["-s", serial, "shell", line.as_str()],
            Duration::from_secs(25),
        )
        .await
    {
        Ok(out) => {
            let text = out.combined();
            Probe {
                refused: text.to_ascii_lowercase().contains("refused"),
                text,
                error: None,
            }
        }
        Err(e) => Probe {
            refused: false,
            text: String::new(),
            error: Some(format!("{e}")),
        },
    }
}

/// A host listener that answers with `marker`, so "the guest reached it" is a
/// string in the probe output rather than a silent socket that looks the same as
/// a timeout.
///
/// The accept loop blocks on purpose: on Windows an accepted socket inherits the
/// listener's non-blocking mode, and a non-blocking write to it can fail silently
/// — which is exactly how a reachable listener comes to look like a hung probe.
fn echo_listener(
    listener: TcpListener,
    marker: &'static str,
) -> (
    Arc<AtomicBool>,
    Arc<AtomicUsize>,
    std::thread::JoinHandle<()>,
) {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    let handle = std::thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut socket, from)) => {
                    // Write before anything else: a guest that half-closes the
                    // moment it has sent its request turns a late write into a
                    // reset, and the marker never arrives.
                    let _ = socket.write_all(marker.as_bytes());
                    let _ = socket.flush();
                    let n = count.fetch_add(1, Ordering::Relaxed) + 1;
                    eprintln!("listener {marker}: connection {n} from {from}");
                }
                Err(_) => break,
            }
        }
    });
    (stop, accepted, handle)
}

/// Wake a blocked `accept` so the listener thread can notice the stop flag. It
/// has to be the listener's own address: a v4 connection never wakes a v6 accept.
fn stop_listener(stop: &AtomicBool, addr: &str) {
    stop.store(true, Ordering::Relaxed);
    let _ = TcpStream::connect(addr);
}

/// `adb root` restarts adbd, so every root step is followed by a transport wait.
async fn as_root(adb: &Adb, serial: &str) {
    let _ = adb
        .run(&["-s", serial, "root"], Duration::from_secs(20))
        .await;
    let _ = adb
        .run(&["-s", serial, "wait-for-device"], Duration::from_secs(30))
        .await;
}

async fn as_shell(adb: &Adb, serial: &str) {
    let _ = adb
        .run(&["-s", serial, "unroot"], Duration::from_secs(20))
        .await;
    let _ = adb
        .run(&["-s", serial, "wait-for-device"], Duration::from_secs(30))
        .await;
}

/// Wait until Android's own networking is up after a cold boot.
///
/// `sys.boot_completed` is set before the emulator's network is usable: a guest
/// booted from scratch answers "Network is unreachable" for the first seconds,
/// which would make the probes below look like a blocked guest. Step 4 asserts
/// that the internet works, so waiting for it here is a precondition, not a
/// weakened assertion.
async fn wait_online(adb: &Adb, serial: &str) {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut last = String::new();
    while Instant::now() < deadline {
        match adb
            .run(
                &["-s", serial, "shell", "ping -c 1 -W 2 8.8.8.8"],
                Duration::from_secs(15),
            )
            .await
        {
            Ok(out) if out.code == Some(0) => {
                eprintln!("guest networking is up");
                return;
            }
            Ok(out) => last = out.combined().trim().to_string(),
            Err(err) => last = err.summary,
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    eprintln!("warning: the guest never answered ping within 120 s (last: {last})");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots or reuses the Android emulator"]
async fn the_guest_cannot_reach_the_hosts_loopback() {
    let harness = Harness::start("isolation").await;
    let runtime = Arc::clone(&harness.runtime);
    let store = Arc::clone(harness.store());

    // Two listeners, one per family: the IPv6 gateway is a separate route to the
    // same host loopback, so a v4-only check would pass on a half-fix.
    let v4 = TcpListener::bind("127.0.0.1:0").expect("a host loopback listener");
    let v4_port = v4.local_addr().unwrap().port();
    let v6 = TcpListener::bind("[::1]:0").expect("an IPv6 host loopback listener");
    let v6_port = v6.local_addr().unwrap().port();
    let (stop4, accepted4, thread4) =
        echo_listener(v4.try_clone().expect("clone the listener"), V4_MARKER);
    let (stop6, _accepted6, thread6) =
        echo_listener(v6.try_clone().expect("clone the listener"), "HOST-V6");
    eprintln!("host listeners: 127.0.0.1:{v4_port} and [::1]:{v6_port}");

    // The runtime installs the rules as part of becoming ready.
    runtime
        .ensure_ready(&store, "isolation-live")
        .await
        .expect("the runtime becomes ready");
    let adb = runtime.adb().expect("the SDK is configured");
    let serial = runtime.serial();
    wait_online(&adb, &serial).await;

    let state = runtime
        .host_isolation()
        .expect("becoming ready installs the egress rules");
    eprintln!("isolation: {state:?}");

    let mut failures: Vec<String> = Vec::new();
    if !state.active() {
        failures.push(format!(
            "the runtime reports no active containment: {state:?}"
        ));
    }

    // The rules must outlive the elevated adbd that installed them, or they would
    // be a property of the test rather than of the runtime.
    let uid = adb
        .run(&["-s", &serial, "shell", "id -u"], Duration::from_secs(15))
        .await
        .map(|out| out.stdout.trim().to_string())
        .unwrap_or_default();
    if uid != "2000" {
        failures.push(format!(
            "adbd is running as uid {uid}; the rules were installed but privileges were not dropped"
        ));
    }

    // 1. Control: the listener really answers, from the host.
    let control = TcpStream::connect(("127.0.0.1", v4_port)).and_then(|mut socket| {
        let mut buf = [0u8; 32];
        let read = socket.read(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf[..read]).to_string())
    });
    match control {
        Ok(text) if text.contains(V4_MARKER) => {
            eprintln!("control: the host reaches 127.0.0.1:{v4_port} and gets {text:?}")
        }
        other => failures.push(format!(
            "the control listener did not answer as expected: {other:?}"
        )),
    }

    // 2. The same port from the guest, both families.
    let v4_probe = probe(&adb, &serial, "10.0.2.2", v4_port).await;
    eprintln!("guest → 10.0.2.2:{v4_port}: {:?}", v4_probe.text.trim());
    if let Some(error) = &v4_probe.error {
        failures.push(format!("the IPv4 probe could not run: {error}"));
    } else if !v4_probe.refused {
        failures.push(format!(
            "a guest still reached the host loopback on 10.0.2.2:{v4_port}: {}",
            v4_probe.text.trim()
        ));
    }

    let v6_probe = probe(&adb, &serial, "fec0::2", v6_port).await;
    eprintln!("guest → [fec0::2]:{v6_port}: {:?}", v6_probe.text.trim());
    if let Some(error) = &v6_probe.error {
        failures.push(format!("the IPv6 probe could not run: {error}"));
    } else if !v6_probe.refused {
        failures.push(format!(
            "a guest still reached the host loopback over IPv6 on [fec0::2]:{v6_port}: {}",
            v6_probe.text.trim()
        ));
    }

    // 3. The measured exposure itself: the adb smart socket.
    let socket = adb.host_probe(&serial).await;
    eprintln!("guest → 10.0.2.2:5037 smart socket: {socket:?}");
    if socket != Some(true) {
        failures.push(format!(
            "the guest's adb smart-socket probe was not refused: {socket:?}"
        ));
    }

    // 4. Networking that is supposed to keep working keeps working.
    let ping = adb
        .run(
            &["-s", &serial, "shell", "ping -c 1 -W 3 8.8.8.8"],
            Duration::from_secs(20),
        )
        .await
        .expect("adb answers");
    eprintln!("guest → 8.8.8.8: exit {:?}", ping.code);
    if ping.code != Some(0) {
        failures.push(format!(
            "the guest lost internet access: {}",
            ping.combined().trim()
        ));
    }

    // 5. Mutation: the refusal has to come from this rule. Removing it must make
    //    the guest reach the listener again, or the checks above prove nothing.
    as_root(&adb, &serial).await;
    let removed = adb
        .run(&["-s", &serial, "shell", V4_RULE], Duration::from_secs(15))
        .await;
    eprintln!("removed the IPv4 rule: {removed:?}");
    let before = accepted4.load(Ordering::Relaxed);
    let open = probe(&adb, &serial, "10.0.2.2", v4_port).await;
    let reached = accepted4.load(Ordering::Relaxed) > before;
    eprintln!(
        "guest → 10.0.2.2:{v4_port} without the rule: {:?} (host accepted: {reached})",
        open.text.trim()
    );
    // Either the marker came back or the host accepted the guest's connection:
    // both mean the guest got through while the rule was missing. If neither
    // happened, the refusals above prove nothing about this rule.
    if !reached && !open.text.contains(V4_MARKER) {
        failures.push(format!(
            "with the rule removed the guest still did not reach the listener ({}), so the refusal above is not this rule's doing",
            open.error.clone().unwrap_or_else(|| open.text.trim().to_string())
        ));
    }
    // Put it back before anything else can run, whatever the verdict.
    let _ = adb
        .run(&["-s", &serial, "shell", V4_ADD], Duration::from_secs(15))
        .await;
    as_shell(&adb, &serial).await;
    let closed = probe(&adb, &serial, "10.0.2.2", v4_port).await;
    eprintln!(
        "guest → 10.0.2.2:{v4_port} with the rule back: {:?}",
        closed.text.trim()
    );
    if !closed.refused {
        failures.push(format!(
            "the rule could not be reinstated: {}",
            closed
                .error
                .clone()
                .unwrap_or_else(|| closed.text.trim().to_string())
        ));
    } // 6. A reboot of Android takes the rules with the kernel that held them. The
      //    runtime has to notice the new boot and put them back, or its status would
      //    keep claiming containment that no longer exists — the failure mode that
      //    makes a security fix worse than none.
    let first_boot = adb.boot_id(&serial).await;
    let _ = adb
        .run(&["-s", &serial, "reboot"], Duration::from_secs(20))
        .await;
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut rebooted = false;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let boot = adb.boot_id(&serial).await;
        if boot.is_none() || boot == first_boot {
            continue;
        }
        let online = adb
            .run(
                &["-s", &serial, "shell", "ping -c 1 -W 2 8.8.8.8"],
                Duration::from_secs(20),
            )
            .await
            .map(|out| out.code == Some(0))
            .unwrap_or(false);
        if online {
            eprintln!("guest rebooted: boot id changed, network back");
            rebooted = true;
            break;
        }
    }
    if !rebooted {
        failures.push(
            "the guest did not come back within 180 s of a reboot, so the reinstall path was not exercised"
                .to_string(),
        );
    } else {
        // The rules are the kernel's, so the new boot must be wide open again.
        let open_again = adb.host_probe(&serial).await;
        eprintln!("after the reboot, before the runtime re-checks: {open_again:?}");
        if open_again != Some(false) {
            failures.push(format!(
                "the rules survived a reboot ({open_again:?}); this test's reboot step no longer proves the reinstall"
            ));
        }
        runtime
            .ensure_ready(&store, "isolation-after-reboot")
            .await
            .expect("the runtime is ready again after a guest reboot");
        let reinstated = runtime.host_isolation();
        eprintln!("isolation after the reboot: {reinstated:?}");
        if !reinstated
            .as_ref()
            .map(|state| state.active())
            .unwrap_or(false)
        {
            failures.push(format!(
                "the runtime did not reinstate containment after the reboot: {reinstated:?}"
            ));
        }
        if adb.host_probe(&serial).await != Some(true) {
            failures.push("the adb socket was reachable after the reboot reinstall".to_string());
        }
    }

    // The emulator is left as it was found: rules in place, adbd unprivileged.
    let after = adb.host_probe(&serial).await;
    if after != Some(true) {
        failures.push(format!(
            "the adb socket was reachable at the end: {after:?}"
        ));
    }

    stop_listener(&stop4, &format!("127.0.0.1:{v4_port}"));
    stop_listener(&stop6, &format!("[::1]:{v6_port}"));
    let _ = thread4.join();
    let _ = thread6.join();
    harness.shutdown().await;

    assert!(
        failures.is_empty(),
        "host isolation failed:\n  - {}",
        failures.join("\n  - ")
    );
}
