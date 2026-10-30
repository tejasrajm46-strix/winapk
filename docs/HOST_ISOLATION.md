# Host isolation: what the guest could reach, and what it can now

Measured on this machine: Windows 11 (26200), AVD `DroidWin_API36`, emulator 36,
`system-images;android-36;google_apis;x86_64` (`ro.debuggable=1`, so `adb root` works).

## 1. Before: what a guest app could reach

The emulator's user-mode networking forwards the guest's host gateway to the **Windows
loopback interface**. `10.0.2.2` is the IPv4 gateway; `fec0::2` is the same gateway over
IPv6. Neither is a hypothetical, and both were probed from inside Android with the same
tool a hostile app has — a socket:

| Probe from inside the guest | Result before |
| --- | --- |
| `printf '000chost:version' \| toybox nc 10.0.2.2 5037` | `OKAY0004…` — the Windows ADB server answered the smart socket |
| `printf 'PING' \| toybox nc 10.0.2.2 41999` (a plain `TcpListener` on `127.0.0.1`) | `PROBE-LOOPBACK` — **any** host listener, not just adb |
| `printf 'PING' \| toybox nc -6 fec0::2 41997` (a listener on `::1`) | `PROBE-V6` — **the IPv6 gateway reaches host loopback too** |
| `printf 'PING' \| toybox nc 10.0.2.2 41998` (nothing listening) | `Connection timeout` |

The third row is why a v4-only fix would have been theatre: blocking `10.0.2.2` alone
leaves every host loopback service reachable through `fec0::2`.

Other surfaces enumerated at the same time:

| Surface | State |
| --- | --- |
| Files or media shared into the AVD | None. `launch_args` mounts no host folder, and Android's `/sdcard` is the AVD's own image. |
| Clipboard | scrcpy 5.0.1 autosyncs the **device** clipboard to Windows whenever the guest changes it. That is a guest → host write path. |
| `reveal_path` / file pull | Host-initiated only: the user picks a package and WINAPK pulls from `/sdcard`. A guest cannot call it, so it is not a guest-reachable host surface. |
| Host command execution | No path was found. The guest could reach *services* (above), not run host commands. |

## 2. The containment

Two egress rules in the guest's netfilter, installed once per boot:

```text
iptables  -I OUTPUT 1 -d 10.0.2.2  -j REJECT     # IPv4 host gateway
ip6tables -I OUTPUT 1 -d fec0::2   -j REJECT     # IPv6 host gateway
```

They sit at position 1 of `OUTPUT`, ahead of Android's own `oem_out`/`fw_OUTPUT` chains, so
they apply to every UID — including an app's `untrusted_app` UID — not just to the shell.
Only the two gateway addresses are refused: DNS (`10.0.2.3`) and the internet are untouched.

Why this shape:

* **Root is needed to change netfilter**, so `adb root` is used once at boot. The rules live
  in the guest kernel, so `adb unroot` follows immediately: the block survives, the elevated
  adbd does not. After installation `id -u` is `2000` (shell).
* **`iptables -C` is the source of truth**, not the insert command's exit code: the runtime
  re-checks that the rule is in the kernel and only then reports containment.
* **A real probe is the second source of truth**: the runtime asks `10.0.2.2:5037` from
  inside the guest and only calls the guest contained when the socket was *refused*. Silence
  (a timeout, a missing `nc`) is recorded as unknown, never as success.
* Nothing in WINAPK's own control path changes: adb, scrcpy, screenshots, recording,
  installs and input all run host-side or over the emulator's transport, never over the
  guest's network.

On a production (Play) image `adb root` is refused. The rules cannot be installed there, so
the runtime reports `Guest access to Windows loopback is NOT contained: …` in its status
instead of claiming the mitigation.

## 3. After: the same probes

From the running guest, with the rules installed and adbd back to the shell user:

| Probe | Result after |
| --- | --- |
| `10.0.2.2:5037` smart socket | `nc: connect: Connection refused` |
| `10.0.2.2:41999` plain listener | `nc: connect: Connection refused` |
| `[fec0::2]:41997` IPv6 listener | `nc: connect: Connection refused` |
| `ping -c1 8.8.8.8` | 1 packet, 0% loss (23.9 ms) |

`tests/isolation_live.rs` repeats this on every live run (`node scripts/live.mjs`, area
`isolation`), and adds three things a one-off probe cannot:

- it asserts `adbd` is back to uid 2000, so the block is a property of the guest and not of a
  privileged shell;
- it removes the IPv4 rule and requires the host listener to *accept* the guest's connection
  (`host accepted: true`), which is what makes the earlier refusals attributable to this rule;
- it reinstates the rule and requires the refusal to return, then checks `adbd` and the socket
  one last time so a failed run cannot leave the machine less protected than it found it;
- it **reboots Android** and requires the guest to be open again, then requires the runtime to notice
  the new kernel boot (`/proc/sys/kernel/random/boot_id`) and reinstate both rules. That is the
  failure mode that would otherwise let the status keep claiming containment the guest had lost.

One quirk is recorded rather than explained away: when the rule is absent, the guest's
connection reaches the host and the listener answers, but `nc` inside the guest does not always
print the reply before the shell command's own timeout ends it. The test therefore judges
reachability by the host's accept, which is not ambiguous; a hand probe with a plain host
listener did print `PROBE-LOOPBACK` in the guest.

## 4. Also closed: the clipboard path

`WindowRequest::arguments` now passes `--no-clipboard-autosync`, so a guest app that changes
Android's clipboard can no longer overwrite the Windows clipboard. WINAPK already reports
clipboard sharing as unverified; this makes the behaviour match the claim.

## 5. What remains

* **The host's LAN address is still reachable.** A guest can open a socket to the machine's
  `192.168.x.x` address, where a service bound to `0.0.0.0` would answer. Blocking that needs
  a Windows Firewall rule (administrator rights, machine-wide), which WINAPK does not
  install. Untrusted packages still belong in a separate Windows account or a VM.
* **Play images refuse `adb root`.** On those, no containment is installed and the status
  says so.* **A reboot of Android discards the rules** — the kernel that held them is gone. The runtime keys
  the containment to the guest's boot id and reinstalls it on the next `ensure_ready`, which the live
  check now exercises with a real reboot. The exposure window is therefore from the reboot until the
  next install, launch or explicit start; a status poll alone does not reinstall.
* **Not a VM.** All apps still share one Android user profile and one kernel; this closes
  the host-loopback path, not app-to-app isolation inside Android.
