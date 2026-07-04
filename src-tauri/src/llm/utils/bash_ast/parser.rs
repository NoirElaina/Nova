// tree-sitter-bash 解析器：把命令字符串解析成 SimpleCommand 列表。
//
// 核心安全属性：fail-closed。任何不在显式 allowlist 中的节点类型都触发
// TooComplex，调用方必须 ask 用户。这意味着我们永远不会去解释不理解
// 的结构。

use crate::llm::utils::bash_ast::types::{ParseForSecurityResult, Redirect, RedirectOp, SimpleCommand};
use tree_sitter::{Node, Parser};
use tree_sitter_bash::LANGUAGE;

/// 结构性节点类型：这些节点可以递归遍历以找到叶子 command 节点。
const STRUCTURAL_TYPES: &[&str] = &[
    "program",
    "list",
    "pipeline",
    "redirected_statement",
];

/// 分隔符节点类型：list/pipeline/program 中命令之间的叶子节点。
const SEPARATOR_TYPES: &[&str] = &["&&", "||", "|", ";", "&", "|&", "\n"];

/// 参数类型 allowlist：这些节点类型可以安全地作为命令参数。
const ARGUMENT_TYPES: &[&str] = &["word", "string", "raw_string", "number"];

/// 命令类型：command 和 declaration_command（export/declare/local 等）。
const COMMAND_TYPES: &[&str] = &["command", "declaration_command"];

/// 重定向操作符映射
fn redirect_op(op: &str) -> Option<RedirectOp> {
    match op {
        ">" => Some(RedirectOp::Out),
        ">>" => Some(RedirectOp::Append),
        "<" => Some(RedirectOp::In),
        ">&" => Some(RedirectOp::OutAnd),
        "<&" => Some(RedirectOp::InAnd),
        ">|" => Some(RedirectOp::OutClobber),
        "&>" => Some(RedirectOp::AndOut),
        "&>>" => Some(RedirectOp::AndAppend),
        "<<<" => Some(RedirectOp::Here),
        _ => None,
    }
}

// 全局 Parser 实例（tree-sitter Parser 不是 Send/Sync，用 thread_local）。
thread_local! {
    static PARSER: std::cell::RefCell<Option<Parser>> = std::cell::RefCell::new(None);
}

/// 初始化或复用 Parser。
fn with_parser<F, R>(f: F) -> R
where
    F: FnOnce(&mut Parser) -> R,
{
    PARSER.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let parser = borrow.get_or_insert_with(|| {
            let mut p = Parser::new();
            p.set_language(&LANGUAGE.into())
                .expect("tree-sitter-bash language load failed");
            p
        });
        f(parser)
    })
}

/// 预检查：在 tree-sitter 解析前先排除已知的 tree-sitter/bash 差分字符。
/// 这些字符会导致 tree-sitter 和 bash 对词边界判断不一致。
fn pre_check(cmd: &str) -> Result<(), String> {
    // 控制字符（不含 \t \n \r，这些是合法的 shell 分隔符）
    for c in cmd.chars() {
        if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' {
            return Err("包含控制字符".to_string());
        }
        if c as u32 == 0x7f {
            return Err("包含控制字符".to_string());
        }
    }

    // Unicode 隐形空白：NBSP、零宽空格、行/段分隔符、BOM
    for c in cmd.chars() {
        let cp = c as u32;
        if cp == 0x00a0
            || cp == 0x1680
            || (0x2000..=0x200b).contains(&cp)
            || cp == 0x2028
            || cp == 0x2029
            || cp == 0x202f
            || cp == 0x205f
            || cp == 0x3000
            || cp == 0xfeff
        {
            return Err("包含 Unicode 隐形空白".to_string());
        }
    }

    // 反斜杠后接空白：tree-sitter 保留原始文本，bash 把 `\ ` 当字面空格
    let bytes = cmd.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t') {
            return Err("包含反斜杠转义的空白".to_string());
        }
        i += 1;
    }

    // zsh ~[ 动态目录展开（调用 hook 执行任意代码）
    if cmd.contains("~[") {
        return Err("包含 zsh ~[ 动态目录语法".to_string());
    }

    // zsh =cmd 等号展开（展开为命令的绝对路径）
    let bytes = cmd.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            // 检查是否在词首（前面是空白或分隔符）
            let at_word_start = i == 0
                || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b';' | b'&' | b'|');
            if at_word_start && i + 1 < bytes.len() {
                let next = bytes[i + 1];
                if next.is_ascii_alphabetic() || next == b'_' {
                    return Err("包含 zsh =cmd 等号展开".to_string());
                }
            }
        }
        i += 1;
    }

    Ok(())
}

/// 对外入口：解析命令字符串，返回 SimpleCommand 列表或 TooComplex。
pub fn parse_for_security(cmd: &str) -> ParseForSecurityResult {
    if cmd.is_empty() {
        return ParseForSecurityResult::Simple { commands: Vec::new() };
    }

    // 预检查：已知的 tree-sitter/bash 差分
    if let Err(reason) = pre_check(cmd) {
        return ParseForSecurityResult::TooComplex { reason };
    }

    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return ParseForSecurityResult::Simple { commands: Vec::new() };
    }

    // tree-sitter 解析
    let tree = with_parser(|parser| parser.parse(cmd, None));
    let Some(tree) = tree else {
        return ParseForSecurityResult::ParseUnavailable;
    };

    let root = tree.root_node();

    // 检查是否有 ERROR 节点（解析失败）
    if root.has_error() {
        return ParseForSecurityResult::TooComplex {
            reason: "命令解析失败（含 ERROR 节点）".to_string(),
        };
    }

    walk_program(cmd, root)
}

/// 遍历 program 根节点，收集所有简单命令。
fn walk_program(source: &str, root: Node) -> ParseForSecurityResult {
    let mut commands: Vec<SimpleCommand> = Vec::new();
    match collect_commands(source, root, &mut commands) {
        Ok(()) => ParseForSecurityResult::Simple { commands },
        Err(reason) => ParseForSecurityResult::TooComplex { reason },
    }
}

/// 递归收集叶子 command 节点。任何不在 allowlist 中的节点类型触发 TooComplex。
fn collect_commands(
    source: &str,
    node: Node,
    commands: &mut Vec<SimpleCommand>,
) -> Result<(), String> {
    let node_type = node.kind();

    // 命令节点：提取 argv/env_vars/redirects
    if COMMAND_TYPES.contains(&node_type) {
        let cmd = walk_command(source, node)?;
        commands.push(cmd);
        return Ok(());
    }

    // 重定向语句：遍历子节点
    if node_type == "redirected_statement" {
        return walk_children(source, node, commands);
    }

    // 注释节点：忽略
    if node_type == "comment" {
        return Ok(());
    }

    // 结构性节点：递归遍历子节点
    if STRUCTURAL_TYPES.contains(&node_type) {
        return walk_children(source, node, commands);
    }

    // negated_command: `! cmd` 只反转退出码，递归到内部命令
    if node_type == "negated_command" {
        return walk_children(source, node, commands);
    }

    // 其他所有节点类型 → fail-closed
    Err(format!(
        "包含无法静态分析的节点类型 '{}'",
        node_type
    ))
}

/// 遍历子节点，跳过分隔符。
fn walk_children(
    source: &str,
    node: Node,
    commands: &mut Vec<SimpleCommand>,
) -> Result<(), String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if SEPARATOR_TYPES.contains(&child.kind()) {
            continue;
        }
        if child.kind() == "!" {
            // negated_command 的 ! 前缀
            continue;
        }
        collect_commands(source, child, commands)?;
    }
    Ok(())
}

/// 从 command 节点提取 SimpleCommand。
fn walk_command(source: &str, node: Node) -> Result<SimpleCommand, String> {
    let mut argv: Vec<String> = Vec::new();
    let mut env_vars: Vec<(String, String)> = Vec::new();
    let mut redirects: Vec<Redirect> = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();

        // 命令名或参数
        if ARGUMENT_TYPES.contains(&kind) {
            let text = child_text(source, child);
            // 检查参数内部是否含未处理的展开
            if let Err(reason) = validate_argument(&child, source) {
                return Err(reason);
            }
            argv.push(text);
            continue;
        }

        // 环境变量赋值：variable_assignment 节点
        if kind == "variable_assignment" {
            if let Some((name, value)) = parse_variable_assignment(source, child) {
                env_vars.push((name, value));
            }
            continue;
        }

        // 重定向：file_redirect / heredoc_redirect 等
        if kind.ends_with("_redirect") || kind == "file_redirect" {
            if let Some(redirect) = parse_redirect(source, child) {
                redirects.push(redirect);
            }
            continue;
        }

        // 命令替换：command_substitution → TooComplex（运行时确定输出）
        if kind == "command_substitution" {
            return Err("包含命令替换 $()，运行时确定输出".to_string());
        }

        // 进程替换：process_substitution → TooComplex
        if kind == "process_substitution" {
            return Err("包含进程替换 <>()".to_string());
        }

        // 子 shell：subshell → TooComplex
        if kind == "subshell" {
            return Err("包含子 shell (())".to_string());
        }

        // 复合语句：compound_statement → TooComplex
        if kind == "compound_statement" {
            return Err("包含复合语句 { }".to_string());
        }

        // 控制流：for/while/until/if/case/function → TooComplex
        if matches!(
            kind,
            "for_statement"
                | "while_statement"
                | "until_statement"
                | "if_statement"
                | "case_statement"
                | "function_definition"
                | "test_command"
                | "arith_expression"
                | "brace_expression"
        ) {
            return Err(format!("包含控制流或函数定义 '{}'", kind));
        }

        // 展开类节点 → TooComplex
        if matches!(
            kind,
            "simple_expansion" | "expansion" | "arith_expansion" | "ansi_c_string"
        ) {
            return Err(format!("包含 shell 展开 '{}'", kind));
        }

        // heredoc → TooComplex（多行内容可能含注入）
        if kind == "heredoc_body" {
            return Err("包含 heredoc 内容".to_string());
        }

        // 其他未知节点类型 → fail-closed
        return Err(format!(
            "命令包含无法静态分析的节点类型 '{}'",
            kind
        ));
    }

    if argv.is_empty() && env_vars.is_empty() {
        // 只有重定向或空命令 → 安全
        return Ok(SimpleCommand {
            argv: Vec::new(),
            env_vars,
            redirects,
            text: node_text(source, node),
        });
    }

    Ok(SimpleCommand {
        argv,
        env_vars,
        redirects,
        text: node_text(source, node),
    })
}

/// 验证参数节点：检查是否含未处理的展开。
fn validate_argument(node: &Node, source: &str) -> Result<(), String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if matches!(
            kind,
            "simple_expansion" | "expansion" | "command_substitution" | "process_substitution"
        ) {
            return Err(format!("参数包含 shell 展开 '{}'", kind));
        }
        // 递归检查子节点
        validate_argument(&child, source)?;
    }
    Ok(())
}

/// 解析 variable_assignment 节点，返回 (name, value)。
fn parse_variable_assignment(source: &str, node: Node) -> Option<(String, String)> {
    let text = node_text(source, node);
    let eq_pos = text.find('=')?;
    let name = text[..eq_pos].to_string();
    let value = text[eq_pos + 1..].to_string();
    Some((name, value))
}

/// 解析重定向节点。
fn parse_redirect(source: &str, node: Node) -> Option<Redirect> {
    // tree-sitter-bash 的 file_redirect 结构：<operator> <target>
    let mut cursor = node.walk();
    let mut op: Option<RedirectOp> = None;
    let mut target: Option<String> = None;

    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if let Some(r_op) = redirect_op(kind) {
            op = Some(r_op);
        } else if ARGUMENT_TYPES.contains(&kind) {
            target = Some(child_text(source, child));
        } else if kind == "simple_expansion" || kind == "expansion" {
            // 重定向目标含展开 → 无法静态确定路径
            // 返回一个特殊标记，让上层检查时拒绝
            target = Some(format!("__DYNAMIC__{}", child_text(source, child)));
        }
    }

    let op = op?;
    let target = target.unwrap_or_default();
    Some(Redirect { op, target, fd: None })
}

/// 获取节点的原始文本（含引号）。
fn node_text<'a>(source: &'a str, node: Node) -> String {
    let start = node.start_byte();
    let end = node.end_byte();
    source[start..end].to_string()
}

/// 获取节点的文本（已去除引号标记）。
/// tree-sitter 返回的 text 是原始源码，含引号。我们对 string/raw_string 类型
/// 做简单的引号去除。
fn child_text(source: &str, node: Node) -> String {
    let text = node_text(source, node);
    let kind = node.kind();

    match kind {
        // 单引号字符串：去除首尾单引号，内容原样
        "raw_string" => {
            if text.len() >= 2 && text.starts_with('\'') && text.ends_with('\'') {
                text[1..text.len() - 1].to_string()
            } else {
                text
            }
        }
        // 双引号字符串：去除首尾双引号
        "string" => {
            if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
                text[1..text.len() - 1].to_string()
            } else {
                text
            }
        }
        _ => text,
    }
}
