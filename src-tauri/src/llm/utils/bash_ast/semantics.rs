// 语义检查：对解析后的 SimpleCommand 做语义级安全检查。
//
// 这是 AST 解析之后的第二层防线。AST 解析能保证 argv 是可信的（引号已解析、
// 无命令替换），但 argv[0] 可能是 `eval`、`source`、`bash -c` 等会执行任意
// 字符串的 builtin。这里做：
//   1. wrapper 剥离（timeout/time/nice/nohup/env/stdbuf）
//   2. eval-like builtin 拦截
//   3. subscript-eval builtin 拦截
//   4. zsh 危险 builtin 拦截

use crate::llm::utils::bash_ast::types::SimpleCommand;

/// 会执行任意字符串作为代码的 shell builtin。
/// `eval "rm -rf /"` 的 argv 是 ['eval', 'rm -rf /']，看起来无害但实际执行了字符串。
const EVAL_LIKE_BUILTINS: &[&str] = &[
    "eval",
    "source",
    ".",
    "exec",
    "command",
    "builtin",
    "fc",
    // coproc rm -rf / 会把 rm 当协程启动
    "coproc",
    // zsh precommand modifiers
    "noglob",
    "nocorrect",
    // trap 'cmd' SIGNAL — cmd 在信号/退出时作为 shell 代码执行
    "trap",
    // enable -f /path/lib.so name — dlopen 任意 .so
    "enable",
    // mapfile -C callback / readarray -C callback — callback 每 N 行执行
    "mapfile",
    "readarray",
    // hash -p /path cmd — 污染命令查找缓存
    "hash",
    // bind -x / complete -C / compgen -C — 执行字符串参数
    "bind",
    "complete",
    "compgen",
    // alias name='cmd' — 配合 shopt -s expand_aliases 可执行
    "alias",
    // let EXPR — 算术求值，等价 $(( EXPR ))
    "let",
];

/// 会重新解析 NAME 操作数并算术求值 arr[EXPR] 下标的 builtin。
/// `test -v 'a[$(id)]'` → tree-sitter 看到的是不透明叶子，bash 会执行 id。
/// 映射：builtin 名 → 哪些 flag 的下一个参数是 NAME。
fn subscript_eval_flags(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "test" | "[" | "[[" => Some(&["-v", "-R"]),
        "printf" => Some(&["-v"]),
        "read" => Some(&["-a"]),
        "unset" => Some(&["-v"]),
        "wait" => Some(&["-p"]),
        _ => None,
    }
}

/// 把每个 bare positional 都当 NAME 重新解析的 builtin。
/// `read 'a[$(id)]' <<< data` 会执行 id，即使 argv[1] 来自单引号。
const BARE_SUBSCRIPT_NAME_BUILTINS: &[&str] = &["read", "unset"];

/// read 的数据接收 flag（后面的参数是 prompt 等，不是 NAME）。
const READ_DATA_FLAGS: &[&str] = &["-p", "-d", "-r", "-s", "-n", "-N", "-t", "-u"];

/// zsh 模块 builtin：通过 zmodload 加载的内部命令。
const ZSH_DANGEROUS_BUILTINS: &[&str] = &[
    "zmodload",
    "emulate",
    "sysopen",
    "sysread",
    "syswrite",
    "sysseek",
    "zpty",
    "ztcp",
    "zsocket",
    "zf_rm",
    "zf_mv",
    "zf_ln",
    "zf_chmod",
    "zf_chown",
    "zf_mkdir",
    "zf_rmdir",
    "zf_chgrp",
];

/// [[ ... ]] 中的算术比较操作符。bash 手册：使用 [[ 时，Arg1 和 Arg2 被作为
/// 算术表达式求值。算术求值会递归展开数组下标，所以 `[[ 'a[$(id)]' -eq 0 ]]`
/// 会执行 id。
const TEST_ARITH_CMP_OPS: &[&str] = &[
    "-eq", "-ne", "-lt", "-le", "-gt", "-ge",
];

/// 对 SimpleCommand 做语义检查。
///
/// 返回 Ok(()) 表示命令语义安全（不含会执行任意代码的 builtin）。
/// 返回 Err(reason) 表示命令无法静态证明安全，应当询问用户。
pub fn check_semantics(cmd: &SimpleCommand) -> Result<(), String> {
    // 剥离 wrapper 命令（timeout/time/nice/nohup/env/stdbuf），
    // 检查被包装的真实命令。
    let stripped_argv = strip_wrappers_from_argv(&cmd.argv);

    if stripped_argv.is_empty() {
        return Ok(());
    }

    let name = &stripped_argv[0];

    // 空命令名：可能是未展开的 $V，bash 会跳过空字段并执行后续命令
    if name.is_empty() {
        return Err("命令名为空，argv[0] 可能不反映 bash 实际执行的命令".to_string());
    }

    // argv[0] 以操作符/flag 开头：可能是片段
    if name.starts_with('-') || name.starts_with('|') || name.starts_with('&') {
        return Err("命令看起来是不完整的片段".to_string());
    }

    // 拦截 eval-like builtin
    if EVAL_LIKE_BUILTINS.contains(&name.as_str()) {
        return Err(format!(
            "命令 '{}' 会将参数作为 shell 代码执行，无法静态分析",
            name
        ));
    }

    // 拦截 zsh 危险 builtin
    if ZSH_DANGEROUS_BUILTINS.contains(&name.as_str()) {
        return Err(format!(
            "命令 '{}' 是 zsh 模块 builtin，可能执行任意操作",
            name
        ));
    }

    // 检查 subscript-eval flag builtin
    if let Some(flags) = subscript_eval_flags(name) {
        let mut i = 1;
        while i < stripped_argv.len() {
            let arg = &stripped_argv[i];
            if flags.contains(&arg.as_str()) {
                // 下一个参数是 NAME，检查是否含数组下标
                if i + 1 < stripped_argv.len() {
                    let next = &stripped_argv[i + 1];
                    if next.contains('[') {
                        return Err(format!(
                            "'{} {} {}' 的 NAME 含数组下标，bash 会算术求值其中的 $(cmd)",
                            name, arg, next
                        ));
                    }
                }
                i += 2;
            } else {
                i += 1;
            }
        }
    }

    // 检查 bare subscript name builtin（read/unset 的每个 bare positional）
    if BARE_SUBSCRIPT_NAME_BUILTINS.contains(&name.as_str()) {
        let mut skip_next = false;
        let mut i = 1;
        while i < stripped_argv.len() {
            let arg = &stripped_argv[i];
            if skip_next {
                skip_next = false;
                i += 1;
                continue;
            }
            if arg.starts_with('-') {
                if name == "read" {
                    if READ_DATA_FLAGS.contains(&arg.as_str()) {
                        skip_next = true;
                    } else if arg.len() > 2 && !arg.starts_with("--") {
                        // 组合短 flag 如 -rp：getopt 风格
                        let chars: Vec<char> = arg.chars().collect();
                        for j in 1..chars.len() {
                            let flag = format!("-{}", chars[j]);
                            if READ_DATA_FLAGS.contains(&flag.as_str()) {
                                if j == chars.len() - 1 {
                                    skip_next = true;
                                }
                                break;
                            }
                        }
                    }
                }
                i += 1;
                continue;
            }
            if arg.contains('[') {
                return Err(format!(
                    "'{}' 的位置参数 NAME '{}' 含数组下标，bash 会算术求值其中的 $(cmd)",
                    name, arg
                ));
            }
            i += 1;
        }
    }

    // 检查 [[ ... ]] 的算术比较操作符两侧
    if name == "[[" {
        let mut i = 2;
        while i < stripped_argv.len() {
            if TEST_ARITH_CMP_OPS.contains(&stripped_argv[i].as_str()) {
                let prev = stripped_argv.get(i - 1).map(|s| s.as_str()).unwrap_or("");
                let next = stripped_argv.get(i + 1).map(|s| s.as_str()).unwrap_or("");
                if prev.contains('[') || next.contains('[') {
                    return Err(format!(
                        "'[[ ... {} ... ]]' 操作数含数组下标，bash 会算术求值其中的 $(cmd)",
                        stripped_argv[i]
                    ));
                }
            }
            i += 1;
        }
    }

    Ok(())
}

/// 从 argv 剥离 wrapper 命令（timeout/time/nice/nohup/env/stdbuf）。
/// 返回剥离后的 argv，argv[0] 是被包装的真实命令。
pub fn strip_wrappers_from_argv(argv: &[String]) -> Vec<String> {
    let mut a: Vec<String> = argv.to_vec();

    loop {
        if a.is_empty() {
            break;
        }
        let name = a[0].as_str();

        if name == "time" || name == "nohup" {
            a = a[1..].to_vec();
        } else if name == "timeout" {
            match skip_timeout_flags(&a) {
                Some(idx) => {
                    // 跳过 flags 和 duration
                    if idx < a.len() {
                        a = a[idx..].to_vec();
                    } else {
                        // timeout 后面没有命令，inert
                        break;
                    }
                }
                None => return Vec::new(), // 无法解析 → 返回空让上层 fail-closed
            }
        } else if name == "nice" {
            // nice cmd / nice -n N cmd / nice -N cmd
            if a.len() >= 3 && a[1] == "-n" && a[2].parse::<i32>().is_ok() {
                a = a[3..].to_vec();
            } else if a.len() >= 2 && a[1].starts_with('-') && a[1][1..].parse::<i32>().is_ok() {
                a = a[2..].to_vec();
            } else if a.len() >= 2 && (a[1].contains('$') || a[1].contains('(') || a[1].contains('`')) {
                // nice 的参数含展开，无法确定被包装命令
                return Vec::new();
            } else {
                a = a[1..].to_vec();
            }
        } else if name == "env" {
            // env [VAR=val...] [-i] [-0] [-v] [-u NAME] cmd
            let mut i = 1;
            while i < a.len() {
                let arg = &a[i];
                if arg.contains('=') && !arg.starts_with('-') {
                    i += 1; // VAR=val
                } else if arg == "-i" || arg == "-0" || arg == "-v" {
                    i += 1;
                } else if arg == "-u" && i + 1 < a.len() {
                    i += 2;
                } else if arg.starts_with('-') {
                    // -S（argv 拆分器）、-C（换 cwd）、-P（换 PATH）或未知 flag
                    return Vec::new();
                } else {
                    break;
                }
            }
            if i < a.len() {
                a = a[i..].to_vec();
            } else {
                break;
            }
        } else if name == "stdbuf" {
            // stdbuf -o0 cmd / stdbuf -o 0 cmd / --output=MODE
            let mut i = 1;
            while i < a.len() {
                let arg = &a[i];
                // -o / -i / -e 后跟 MODE（空格分隔）
                if (arg == "-o" || arg == "-i" || arg == "-e") && i + 1 < a.len() {
                    i += 2;
                }
                // 融合形式：-o0、-eL
                else if arg.starts_with("-o")
                    || arg.starts_with("-i")
                    || arg.starts_with("-e")
                {
                    i += 1;
                }
                // 长形式：--output=MODE
                else if arg.starts_with("--output=")
                    || arg.starts_with("--input=")
                    || arg.starts_with("--error=")
                {
                    i += 1;
                } else if arg.starts_with('-') {
                    return Vec::new();
                } else {
                    break;
                }
            }
            if i > 1 && i < a.len() {
                a = a[i..].to_vec();
            } else {
                break;
            }
        } else {
            break;
        }
    }

    a
}

/// 解析 timeout 的 GNU flags，返回 duration token 的 argv 索引。
/// 返回 None 表示 flags 无法静态解析（含展开或未知 flag）。
fn skip_timeout_flags(a: &[String]) -> Option<usize> {
    let mut i = 1;
    while i < a.len() {
        let arg = &a[i];
        if arg == "--foreground" || arg == "--preserve-status" || arg == "--verbose" {
            i += 1;
        } else if arg.starts_with("--kill-after=") || arg.starts_with("--signal=") {
            // 值必须匹配 allowlist [A-Za-z0-9_.+-]+
            let value = arg.splitn(2, '=').nth(1)?;
            if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '+' || c == '-') {
                return None;
            }
            i += 1;
        } else if (arg == "--kill-after" || arg == "--signal") && i + 1 < a.len() {
            let value = &a[i + 1];
            if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '+' || c == '-') {
                return None;
            }
            i += 2;
        } else if arg.starts_with("--") {
            return None; // 未知长 flag
        } else if arg == "-v" {
            i += 1;
        } else if (arg == "-k" || arg == "-s") && i + 1 < a.len() {
            let value = &a[i + 1];
            if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '+' || c == '-') {
                return None;
            }
            i += 2;
        } else if arg.starts_with("-k") || arg.starts_with("-s") {
            // 融合形式：-k5、-sTERM
            let value = &arg[2..];
            if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '+' || c == '-') {
                return None;
            }
            i += 1;
        } else if arg.starts_with('-') {
            return None; // 未知短 flag
        } else {
            break; // 非 flag，应该是 duration
        }
    }

    // 检查 duration token
    if i < a.len() {
        let dur = &a[i];
        // 接受 5、5s、5.5、10m 等形式
        if is_valid_duration(dur) {
            Some(i + 1)
        } else {
            None // duration 无法静态解析
        }
    } else {
        Some(i) // timeout 后面没有 duration（inert）
    }
}

/// 检查是否是合法的 timeout duration（5、5s、5.5、10m 等）。
fn is_valid_duration(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let bytes = s.as_bytes();
    let mut end = bytes.len();

    // 可选的后缀 s/m/h/d
    if let Some(last) = bytes.last() {
        if matches!(last, b's' | b'm' | b'h' | b'd') {
            end -= 1;
        }
    }

    let num = &s[..end];
    if num.is_empty() {
        return false;
    }

    // 接受整数或小数
    num.parse::<f64>().is_ok()
}
