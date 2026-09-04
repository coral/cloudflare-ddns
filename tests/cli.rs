use std::process::Command;

const VALID_ZONE_ID: &str = "0123456789abcdef0123456789abcdef";

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cf-ddns"));
    command
        .env("CLOUDFLARE_API_TOKEN", "super-secret-token")
        .env("CLOUDFLARE_ZONE_ID", VALID_ZONE_ID)
        .env("CLOUDFLARE_RECORD_NAME", "home.example.com");
    command
}

#[test]
fn command_line_values_override_environment() {
    let output = command()
        .arg("--zone-id")
        .arg("invalid-command-line-zone")
        .arg("--once")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("zone ID must contain exactly 32 hexadecimal characters")
    );
}

#[test]
fn help_never_prints_the_token_value() {
    let output = command().arg("--help").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("CLOUDFLARE_API_TOKEN"));
    assert!(!stdout.contains("super-secret-token"));
}
