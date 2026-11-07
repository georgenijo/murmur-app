//! Real passive HAL-listener lifecycle; never opens a microphone or enumerates devices.
#![cfg(all(target_os = "macos", target_arch = "aarch64"))]
use murmur_capture_helper_protocol::{
    read_production_frame, write_production_control, ProductionFrame, ProductionHelperMessage,
    ProductionHostMessage,
};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct OwnedHelper(Child);
impl Drop for OwnedHelper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "registers real passive Core Audio listeners on this Mac; no capture"]
fn passive_topology_watch_stops_cancels_and_reaps_on_stdin_eof() {
    for command in [
        Some(ProductionHostMessage::Stop),
        Some(ProductionHostMessage::Cancel),
        None,
    ] {
        let mut helper = OwnedHelper(
            Command::new(env!("CARGO_BIN_EXE_murmur-capture-helper"))
                .args(["--production-v10", "73", "01010101010101010101010101010101"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut input = helper.0.stdin.take().unwrap();
        let mut output = helper.0.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || loop {
            let frame = read_production_frame::<ProductionHelperMessage>(&mut output, 73, [1; 16]);
            let eof = frame.is_err();
            if tx.send(frame).is_err() || eof {
                return;
            }
        });
        let send = |input: &mut std::process::ChildStdin, message| {
            write_production_control(input, 73, [1; 16], &message).unwrap();
        };
        send(&mut input, ProductionHostMessage::Hello);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap(),
            ProductionFrame::Control(ProductionHelperMessage::HelloAck)
        ));
        send(&mut input, ProductionHostMessage::WatchInputTopology);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap(),
            ProductionFrame::Control(ProductionHelperMessage::InputTopologyWatchReady)
        ));
        let started = Instant::now();
        let had_command = command.is_some();
        if let Some(command) = command {
            send(&mut input, command);
        }
        drop(input); // Also covers ownership EOF with no shutdown frame.
        let mut stopped = false;
        while let Ok(frame) = rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            match frame {
                ProductionFrame::Control(ProductionHelperMessage::Stopped {
                    retained_samples: 0,
                }) => stopped = true,
                ProductionFrame::Control(ProductionHelperMessage::InputTopologyChanged) => {}
                _ => panic!("unexpected passive watcher frame"),
            }
        }
        let status = helper.0.wait().unwrap();
        assert!(status.success());
        assert_eq!(stopped, had_command);
        reader.join().unwrap();
        println!(
            "passive_topology_lifecycle command={} elapsed_ms={} exit_code={}",
            if had_command {
                "stop_or_cancel"
            } else {
                "stdin_eof"
            },
            started.elapsed().as_millis(),
            status.code().unwrap_or(-1)
        );
    }
}
