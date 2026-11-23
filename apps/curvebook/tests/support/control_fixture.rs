//! Process deadlines and acknowledged cleanup for real TCP control fixtures.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use curvebook_platform::process::ProcessCommand;
use serde_json::{Value, json};

pub type TestResult = Result<(), Box<dyn std::error::Error>>;
const CHILD_TEST: &str = "CURVEBOOK_CONTROL_FIXTURE_CHILD";

pub fn is_child(name: &str) -> bool {
    std::env::var(CHILD_TEST).is_ok_and(|value| value == name)
}

pub fn run_child(name: &str, timeout: Duration) -> TestResult {
    let executable = std::env::current_exe()?;
    let output = ProcessCommand::new(executable.to_string_lossy())
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(CHILD_TEST, name)
        .env("CURVEBOOK_NO_PREFS", "1")
        .wall_timeout(timeout)
        .idle_timeout(timeout)
        .max_output_bytes(1024 * 1024)
        .run()?;
    if !output.success {
        return Err(format!(
            "TCP fixture {name} failed (exit {:?}):\n{}\n{}",
            output.status_code,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

pub fn run(name: &str, body: impl FnOnce() -> TestResult) -> TestResult {
    if is_child(name) { body() } else { run_child(name, Duration::from_secs(45)) }
}

/// Declared before client sockets so their drops release the active connection first.
/// An early error or panic then stops the listener before the test joins the client.
pub struct QuitOnFailure {
    address: SocketAddr,
    token: String,
    armed: bool,
}

impl QuitOnFailure {
    pub fn new(address: SocketAddr, token: &str) -> Self {
        Self { address, token: token.to_owned(), armed: true }
    }

    pub fn disarm(&mut self) {
        self.armed = false;
    }

    fn stop(&self) -> TestResult {
        let timeout = Duration::from_secs(2);
        let mut stream = TcpStream::connect_timeout(&self.address, timeout)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let request = json!({"id": "fixture-cleanup", "method": "app.quit", "token": self.token});
        writeln!(stream, "{request}")?;
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response)?;
        let response: Value = serde_json::from_str(&response)?;
        if response.get("result").and_then(|r| r.get("quit")) != Some(&json!(true)) {
            return Err("TCP fixture cleanup did not acknowledge app.quit".into());
        }
        Ok(())
    }
}

impl Drop for QuitOnFailure {
    fn drop(&mut self) {
        if self.armed
            && let Err(err) = self.stop()
        {
            eprintln!("TCP fixture cleanup failed: {err}; the subprocess deadline remains enforced");
        }
    }
}
