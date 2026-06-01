use std::path::Path;

use anyhow::Result;
use predicates::str::contains;
use tempfile::TempDir;

fn codex_command(codex_home: &Path) -> Result<assert_cmd::Command> {
    let mut cmd = assert_cmd::Command::new(codex_utils_cargo_bin::cargo_bin("codex")?);
    cmd.env("CODEX_HOME", codex_home);
    Ok(cmd)
}

fn write_file_auth_config(codex_home: &Path) -> Result<()> {
    std::fs::write(
        codex_home.join("config.toml"),
        "cli_auth_credentials_store = \"file\"\n",
    )?;
    Ok(())
}

#[test]
fn login_prints_deepseek_auth_guidance_without_writing_auth_json() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_file_auth_config(codex_home.path())?;

    let mut cmd = codex_command(codex_home.path())?;
    cmd.arg("login")
        .assert()
        .success()
        .stderr(contains(
            "SeekForge does not use native OpenAI/ChatGPT login",
        ))
        .stderr(contains("DEEPSEEK_API_KEY"))
        .stderr(contains("This command does not create or update auth.json"));

    assert!(!codex_home.path().join("auth.json").exists());

    Ok(())
}

#[test]
fn login_with_api_key_is_disabled_and_does_not_write_auth_json() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_file_auth_config(codex_home.path())?;

    let mut cmd = codex_command(codex_home.path())?;
    cmd.args(["login", "--with-api-key"])
        .write_stdin("sk-test\n")
        .assert()
        .failure()
        .stderr(contains(
            "SeekForge does not support native OpenAI/ChatGPT login or auth.json API-key login",
        ))
        .stderr(contains("DEEPSEEK_API_KEY"))
        .stderr(contains("This command does not create or update auth.json"));

    assert!(!codex_home.path().join("auth.json").exists());

    Ok(())
}

#[test]
fn login_with_access_token_is_disabled_before_jwt_validation() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_file_auth_config(codex_home.path())?;

    let mut cmd = codex_command(codex_home.path())?;
    cmd.args(["login", "--with-access-token"])
        .write_stdin("not-a-jwt\n")
        .assert()
        .failure()
        .stderr(contains(
            "SeekForge does not support native OpenAI/ChatGPT login or auth.json API-key login",
        ));

    assert!(!codex_home.path().join("auth.json").exists());

    Ok(())
}

#[test]
fn login_status_reports_deepseek_env_key() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_file_auth_config(codex_home.path())?;

    let mut cmd = codex_command(codex_home.path())?;
    cmd.args(["login", "status"])
        .env("DEEPSEEK_API_KEY", "sk-test")
        .assert()
        .success()
        .stderr(contains("Active model provider: DeepSeek (deepseek)"))
        .stderr(contains(
            "Provider API key environment variable DEEPSEEK_API_KEY is present",
        ));

    Ok(())
}
