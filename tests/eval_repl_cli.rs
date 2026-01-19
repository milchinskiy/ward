use std::process::{Command, Stdio};

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

#[test]
fn eval_exposes_arg_table() {
    let output = ward_cmd()
        .args(["eval", "-e", "print(_G.arg[0]); print(_G.arg[1])", "--", "foo"])
        .output()
        .expect("eval output");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.first().copied(), Some("(eval)"));
    assert_eq!(lines.get(1).copied(), Some("foo"));
}

#[test]
fn repl_exits_on_exit_command_and_prints_expression() {
    let mut cmd = ward_cmd();
    cmd.args(["repl", "--no-prompt"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("spawn repl");
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(b"=1+2\nexit\n").expect("write stdin");
    }
    let output = child.wait_with_output().expect("wait");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.lines().any(|l| l.trim() == "3"), "stdout was: {stdout}");
}

#[test]
fn repl_exits_on_eof() {
    let mut cmd = ward_cmd();
    cmd.args(["repl", "--no-prompt"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("spawn repl");
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(b"print('hi')\n").expect("write stdin");
        // drop stdin to send EOF
    }
    let output = child.wait_with_output().expect("wait");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hi"), "stdout was: {stdout}");
}
