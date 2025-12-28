use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;

fn ward_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("ward"))
}

fn write_script(temp: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
    let path = temp.path().join(name);
    std::fs::write(&path, body).expect("failed to write lua script");
    path
}

fn run_lua_script(body: &str) -> Value {
    let temp = tempdir().expect("tempdir");
    let script = write_script(&temp, "script.lua", body);

    let output = ward_cmd()
        .args(["run", script.to_string_lossy().as_ref()])
        .output()
        .expect("run output");

    assert!(
        output.status.success(),
        "lua script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).expect("stdout json")
}

#[test]
fn template_minijinja_renders_strings_and_files() {
    let temp = tempdir().expect("tempdir");
    let templates_dir = temp.path().join("templates");
    std::fs::create_dir_all(&templates_dir).expect("templates dir");

    let main_path = templates_dir.join("main.j2");

    std::fs::write(
        &main_path,
        "Header\n{% if item %}\n  Item: {{ item }}\n{% endif %}\nMissing: {{ missing }}\nEscaped: {{ extra }}\n",
    )
    .expect("write main template");

    let main_literal = serde_json::to_string(main_path.to_string_lossy().as_ref()).expect("main literal");

    let script = format!(
        r#"local tpl = require("ward.template.minijinja")
local json = require("ward.convert.json")

local function coerce(val)
  if val == nil then
    return nil
  end
  if type(val) == "string" or type(val) == "boolean" or type(val) == "number" then
    return val
  end
  return tostring(val)
end

local ok_render, rendered = pcall(function()
  return tpl.render("Hello {{{{ who }}}}", {{ who = "templating" }})
end)

local ok_render_async, rendered_async = pcall(function()
  return tpl.render_async("{{{{ x }}}} + {{{{ y }}}} = {{{{ x + y }}}}", {{ x = 2, y = 3 }})
end)

local ok_file, file_output = pcall(function()
  return tpl.render_file({main_literal}, {{
    item = "apple",
    extra = "<ok>",
  }}, {{
    trim_blocks = true,
    lstrip_blocks = true,
    keep_trailing_newline = true,
    auto_escape = true,
    undefined = "lenient",
  }})
end)

print(json.encode({{
  ok_render = ok_render,
  rendered = coerce(rendered),
  ok_render_async = ok_render_async,
  rendered_async = coerce(rendered_async),
  ok_file = ok_file,
  file_output = coerce(file_output),
}}))
"#
    );

    let value = run_lua_script(&script);

    assert_eq!(value["ok_render"], Value::Bool(true), "{value}");
    assert_eq!(value["rendered"], Value::from("Hello templating"));
    assert_eq!(value["ok_render_async"], Value::Bool(true), "{value}");
    assert_eq!(value["rendered_async"], Value::from("2 + 3 = 5"));
    assert_eq!(value["ok_file"], Value::Bool(true), "{value}");
    assert_eq!(
        value["file_output"],
        Value::from("Header\n  Item: apple\nMissing: \nEscaped: &lt;ok&gt;\n")
    );
}
