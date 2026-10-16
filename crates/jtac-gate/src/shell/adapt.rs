//! shuck-ast to `Item`: the only file that names the parser. A parse that is not clean is an
//! error, because a recovered parse means something ran that this reader did not see
//! (DESIGN.md decision 36). Every place the grammar can hold a substitution is visited, since
//! each one runs; a pipe's sides and a backgrounded command are subshells, as in bash.

use shuck_ast::{
    ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArrayElem, AssignmentValue, BinaryOp,
    BourneParameterExpansion, BuiltinCommand, Command, CommandSubstitutionSyntax, CompoundCommand,
    ConditionalBinaryOp, ConditionalExpr, ConditionalUnaryOp, DeclClause, DeclOperand,
    HeredocBodyPart, ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern,
    PatternPart, Redirect, RedirectKind, RedirectTarget, SimpleCommand, Span, Stmt, StmtSeq,
    StmtTerminator, VarRef, Word as AstWord, WordPart, static_command_name_text,
};
use shuck_parser::parser::{ParseStatus, Parser};

use super::quote::{ansi_decode, spells_a_substitution, unescape};
use super::scan::{MAX_SUBSHELLS, too_deep_or_long};

use super::{Item, Simple, Word};

/// The parser recurses once per nested construct and the hook's own stack is not ours to
/// size. The deepest shape under [`crate::classify::MAX_BYTES`] (`time` repeated) takes about
/// 4 KiB of stack per byte in a debug build; this reserves twice that, virtual and untouched
/// until a script needs it (`every_shape_under_the_size_cap_parses_without_aborting`).
const PARSER_STACK: usize = 512 << 20;

/// The script's commands in run order, or why they cannot be read. A parser panic or a
/// thread the system refuses is unreadable, never an abort of the caller.
pub(crate) fn read(script: &str) -> Result<Vec<Item>, String> {
    if let Some(why) = too_deep_or_long(script) {
        return Err(why);
    }
    let owned = script.to_string();
    std::thread::Builder::new()
        .name("jtac-shell-parser".to_string())
        .stack_size(PARSER_STACK)
        .spawn(move || parse(&owned))
        .map_err(|e| format!("the parser could not start a thread: {e}"))?
        .join()
        .map_err(|_| "the parser panicked".to_string())?
}

// bash reads these words again as arithmetic or as an array name, so a quoted `$(…)` in one
// runs: `let 'a[$(cmd)]++'`, `unset 'a[$(cmd)]'`, `printf -v 'a[$(cmd)]' x`, `test -v`.
fn evaluates(prog: &str, args: &[Word], quoted: &[bool]) -> bool {
    let any = quoted.contains(&true);
    let flag = |c: char| {
        args.iter()
            .take_while(|a| a.text.starts_with('-'))
            .any(|a| a.text.contains(c))
    };
    match prog {
        "let" | "unset" | "read" | "compgen" | "wait" => any,
        "printf" => args.iter().enumerate().any(|(k, a)| {
            a.text.strip_prefix("-v").is_some_and(|rest| {
                if rest.is_empty() {
                    quoted.get(k + 1) == Some(&true)
                } else {
                    quoted[k]
                }
            })
        }),
        "test" | "[" => any && args.iter().any(|a| matches!(a.text.as_str(), "-v" | "-R")),
        "declare" | "typeset" | "local" | "readonly" => args.iter().zip(quoted).any(|(a, q)| {
            let (name, value) = a.text.split_once('=').unwrap_or((&a.text, ""));
            let scalar = prog != "readonly" && (name.contains('[') || flag('i'));
            *q && !a.text.starts_with('-')
                && (scalar || (flag('a') || flag('A')) && value.starts_with('('))
        }),
        _ => false,
    }
}

fn parse(script: &str) -> Result<Vec<Item>, String> {
    let parsed = Parser::new(script).parse();
    if parsed.status != ParseStatus::Clean {
        return Err(parsed.diagnostics.first().map_or_else(
            || "the parser stopped without a diagnostic".to_string(),
            |d| d.message.clone(),
        ));
    }
    let mut walk = Walk {
        source: script,
        out: Vec::new(),
        depth: 0,
        too_deep: false,
        escaped_backtick: false,
        quoted_arith: false,
        prompt_expansion: false,
        piped: false,
    };
    walk.seq(&parsed.file.body, false);
    if walk.too_deep {
        return Err(format!(
            "it nests subshells or substitutions more than {MAX_SUBSHELLS} deep"
        ));
    }
    if walk.escaped_backtick {
        return Err("it escapes a backtick inside a backtick body".to_string());
    }
    if walk.quoted_arith {
        return Err(
            "a quoted substitution in an arithmetic body or subscript runs when it is evaluated"
                .to_string(),
        );
    }
    if walk.prompt_expansion {
        return Err(
            "a prompt expansion (`${x@P}`, or `PS4` set here) runs any substitution in the value"
                .to_string(),
        );
    }
    Ok(walk.out)
}

struct Walk<'a> {
    source: &'a str,
    out: Vec<Item>,
    depth: usize,
    too_deep: bool,
    /// shuck keeps `\`` in a backtick body as text; bash opens a substitution there.
    escaped_backtick: bool,
    /// `(( '$(cmd)' ))`, `a['$(cmd)']`: bash expands an arithmetic body or a subscript again
    /// when it evaluates it, so a quoted substitution there runs.
    quoted_arith: bool,
    /// `${x@P}`: bash expands the value as a prompt, substitutions included.
    prompt_expansion: bool,
    /// Stdin may be another command's output: every command on a pipe's right side.
    piped: bool,
}

impl Walk<'_> {
    fn seq(&mut self, seq: &StmtSeq, conditional: bool) {
        for stmt in &seq.stmts {
            self.stmt(stmt, conditional);
        }
    }

    fn stmt(&mut self, stmt: &Stmt, conditional: bool) {
        if matches!(stmt.terminator, Some(StmtTerminator::Background(_))) {
            self.child(|w| w.stmt_here(stmt, conditional));
        } else {
            self.stmt_here(stmt, conditional);
        }
    }

    fn stmt_here(&mut self, stmt: &Stmt, conditional: bool) {
        let heredoc = self.redirects(&stmt.redirects);
        match &stmt.command {
            Command::Simple(c) => self.simple(c, stmt.span, conditional, heredoc),
            Command::Decl(d) => self.decl(d, stmt.span, conditional),
            Command::Builtin(b) => self.builtin(b),
            Command::Binary(b) => match b.op {
                BinaryOp::Pipe | BinaryOp::PipeAll => {
                    self.pipe_side(&b.left, conditional, false);
                    self.pipe_side(&b.right, conditional, true);
                }
                // The right side runs only if the left said so, either way round.
                BinaryOp::And | BinaryOp::Or => {
                    self.stmt(&b.left, conditional);
                    self.stmt(&b.right, true);
                }
            },
            Command::Compound(c) => self.compound(c, conditional),
            Command::Function(f) => self.stmt(&f.body, true),
            Command::AnonymousFunction(a) => {
                self.words(&a.args);
                self.stmt(&a.body, true);
            }
        }
    }

    fn compound(&mut self, c: &CompoundCommand, conditional: bool) {
        match c {
            CompoundCommand::If(i) => {
                self.seq(&i.condition, conditional);
                self.seq(&i.then_branch, true);
                for (cond, body) in &i.elif_branches {
                    self.seq(cond, true);
                    self.seq(body, true);
                }
                if let Some(e) = &i.else_branch {
                    self.seq(e, true);
                }
            }
            CompoundCommand::For(f) => {
                if let Some(words) = &f.words {
                    self.words(words);
                }
                self.seq(&f.body, true);
            }
            CompoundCommand::Repeat(r) => {
                self.word(&r.count);
                self.seq(&r.body, true);
            }
            CompoundCommand::Foreach(f) => {
                self.words(&f.words);
                self.seq(&f.body, true);
            }
            CompoundCommand::ArithmeticFor(a) => {
                for e in [&a.init_ast, &a.condition_ast, &a.step_ast] {
                    self.arith(e.as_ref());
                }
                self.seq(&a.body, true);
            }
            CompoundCommand::While(w) => {
                self.seq(&w.condition, true);
                self.seq(&w.body, true);
            }
            CompoundCommand::Until(u) => {
                self.seq(&u.condition, true);
                self.seq(&u.body, true);
            }
            CompoundCommand::Case(c) => {
                self.word(&c.word);
                for item in &c.cases {
                    for p in &item.patterns {
                        self.pattern(p);
                    }
                    self.seq(&item.body, true);
                }
            }
            CompoundCommand::Select(s) => {
                self.words(&s.words);
                self.seq(&s.body, true);
            }
            CompoundCommand::Subshell(seq) => self.child(|w| w.seq(seq, conditional)),
            CompoundCommand::BraceGroup(seq) => self.seq(seq, conditional),
            CompoundCommand::Time(t) => {
                if let Some(cmd) = &t.command {
                    self.stmt(cmd, conditional);
                }
            }
            CompoundCommand::Coproc(c) => self.child(|w| w.stmt(&c.body, conditional)),
            CompoundCommand::Always(a) => {
                self.seq(&a.body, conditional);
                self.seq(&a.always_body, true);
            }
            CompoundCommand::Conditional(c) => self.conditional(&c.expression),
            CompoundCommand::Arithmetic(a) => self.arith(a.expr_ast.as_ref()),
        }
    }

    // `$(…)` is a primary of arithmetic too; every other node only holds more nodes.
    fn arith(&mut self, node: Option<&ArithmeticExprNode>) {
        let Some(node) = node else { return };
        match &node.kind {
            ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
            ArithmeticExpr::ShellWord(w) => {
                self.arith_word(w);
            }
            ArithmeticExpr::Indexed { index: e, .. }
            | ArithmeticExpr::Parenthesized { expression: e }
            | ArithmeticExpr::Unary { expr: e, .. }
            | ArithmeticExpr::Postfix { expr: e, .. } => self.arith(Some(e)),
            ArithmeticExpr::Binary { left, right, .. } => {
                self.arith(Some(left));
                self.arith(Some(right));
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.arith(Some(condition));
                self.arith(Some(then_expr));
                self.arith(Some(else_expr));
            }
            ArithmeticExpr::Assignment { target, value, .. } => {
                if let ArithmeticLvalue::Indexed { index, .. } = target {
                    self.arith(Some(index));
                }
                self.arith(Some(value));
            }
        }
    }

    // A `\`` in a backtick body is a nested substitution shuck keeps as text.
    fn substitution(&mut self, body: &StmtSeq, syntax: CommandSubstitutionSyntax, span: Span) {
        if syntax == CommandSubstitutionSyntax::Backtick && self.slice(span).contains("\\`") {
            self.escaped_backtick = true;
        }
        self.child(|w| w.seq(body, false));
    }

    // Every stage of a pipeline is a sibling subshell, however the tree nests them.
    fn pipe_side(&mut self, stmt: &Stmt, conditional: bool, piped: bool) {
        match &stmt.command {
            Command::Binary(b) if matches!(b.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
                self.pipe_side(&b.left, conditional, piped);
                self.pipe_side(&b.right, conditional, true);
            }
            _ => self.child(|w| {
                w.piped |= piped;
                w.stmt(stmt, conditional);
            }),
        }
    }

    // A subshell's commands become one `Item`; its `cd` cannot move the parent.
    fn child(&mut self, walk: impl FnOnce(&mut Walk<'_>)) {
        if self.depth >= MAX_SUBSHELLS {
            self.too_deep = true;
            return;
        }
        let mut child = Walk {
            source: self.source,
            out: Vec::new(),
            depth: self.depth + 1,
            too_deep: false,
            escaped_backtick: false,
            quoted_arith: false,
            prompt_expansion: false,
            piped: self.piped,
        };
        walk(&mut child);
        self.too_deep |= child.too_deep;
        self.escaped_backtick |= child.escaped_backtick;
        self.quoted_arith |= child.quoted_arith;
        self.prompt_expansion |= child.prompt_expansion;
        self.out.push(Item::Subshell(child.out));
    }

    fn builtin(&mut self, b: &BuiltinCommand) {
        let (arg, extra, assignments) = match b {
            BuiltinCommand::Exit(e) => (&e.code, &e.extra_args, &e.assignments),
            BuiltinCommand::Return(r) => (&r.code, &r.extra_args, &r.assignments),
            BuiltinCommand::Break(c) => (&c.depth, &c.extra_args, &c.assignments),
            BuiltinCommand::Continue(c) => (&c.depth, &c.extra_args, &c.assignments),
        };
        if let Some(w) = arg {
            self.word(w);
        }
        self.words(extra);
        for a in assignments {
            self.assignment(&a.target, &a.value);
        }
    }

    fn conditional(&mut self, e: &ConditionalExpr) {
        match e {
            ConditionalExpr::Binary(b) => {
                let arith = matches!(
                    b.op,
                    ConditionalBinaryOp::ArithmeticEq
                        | ConditionalBinaryOp::ArithmeticNe
                        | ConditionalBinaryOp::ArithmeticLe
                        | ConditionalBinaryOp::ArithmeticGe
                        | ConditionalBinaryOp::ArithmeticLt
                        | ConditionalBinaryOp::ArithmeticGt
                );
                self.operand(&b.left, arith);
                self.operand(&b.right, arith);
            }
            ConditionalExpr::Unary(u) => self.operand(
                &u.expr,
                matches!(
                    u.op,
                    ConditionalUnaryOp::VariableSet | ConditionalUnaryOp::ReferenceVariable
                ),
            ),
            ConditionalExpr::Parenthesized(p) => self.conditional(&p.expr),
            ConditionalExpr::Word(w) | ConditionalExpr::Regex(w) => {
                self.word(w);
            }
            ConditionalExpr::Pattern(p) => self.pattern(p),
            ConditionalExpr::VarRef(v) => {
                self.reference(v);
            }
        }
    }

    // `[[ 1 -eq 'a[$(cmd)]' ]]`, `[[ -v 'a[$(cmd)]' ]]`: the operand is evaluated as arithmetic
    // or as an array name, so a quoted substitution in it runs.
    fn operand(&mut self, e: &ConditionalExpr, arith: bool) {
        match e {
            ConditionalExpr::Word(w) if arith => {
                self.arith_word(w);
            }
            other => self.conditional(other),
        }
    }

    fn pattern(&mut self, p: &Pattern) {
        for part in &p.parts {
            match &part.kind {
                PatternPart::Word(w) => {
                    self.word(w);
                }
                PatternPart::Group { patterns, .. } => {
                    for p in patterns {
                        self.pattern(p);
                    }
                }
                _ => {}
            }
        }
    }

    fn simple(&mut self, c: &SimpleCommand, span: Span, conditional: bool, heredoc: Option<Word>) {
        let mut words = Vec::with_capacity(c.assignments.len() + 1 + c.args.len());
        for a in &c.assignments {
            words.push(self.assignment(&a.target, &a.value));
        }
        let name = self.word(&c.name);
        let args: Vec<Word> = c.args.iter().map(|w| self.word(w)).collect();
        let quoted: Vec<bool> = c.args.iter().map(|w| self.quoted_substitution(w)).collect();
        let all: Vec<&Word> = std::iter::once(&name).chain(args.iter()).collect();
        let mut i = 0;
        while all
            .get(i)
            .is_some_and(|w| matches!(w.text.as_str(), "builtin" | "command"))
        {
            i += 1;
            while all.get(i).is_some_and(|w| w.text.starts_with('-')) {
                i += 1;
            }
        }
        if let Some(prog) = all.get(i)
            && evaluates(&prog.text, &args[i..], &quoted[i..])
        {
            self.quoted_arith = true;
        }
        if !name.text.is_empty() || name.unresolved {
            words.push(name);
        }
        words.extend(args);
        self.push(words, span, conditional, heredoc);
    }

    // `export K=v` is read as the command `export` with a `K=v` word, so the git scan sees it.
    fn decl(&mut self, d: &DeclClause, span: Span, conditional: bool) {
        let mut words = vec![Word {
            text: d.variant.as_ref().to_string(),
            unresolved: false,
        }];
        let mut quoted = Vec::with_capacity(d.operands.len());
        for op in &d.operands {
            quoted.push(match op {
                DeclOperand::Dynamic(w) => self.quoted_substitution(w),
                DeclOperand::Assignment(a) => {
                    matches!(&a.value, AssignmentValue::Scalar(w) if self.quoted_substitution(w))
                }
                DeclOperand::Flag(_) | DeclOperand::Name(_) => false,
            });
            words.push(match op {
                DeclOperand::Flag(w) | DeclOperand::Dynamic(w) => self.word(w),
                DeclOperand::Name(v) => {
                    self.reference(v);
                    Word {
                        text: v.name.as_ref().to_string(),
                        unresolved: false,
                    }
                }
                DeclOperand::Assignment(a) => self.assignment(&a.target, &a.value),
            });
        }
        self.quoted_arith |= evaluates(d.variant.as_ref(), &words[1..], &quoted);
        self.push(words, span, conditional, None);
    }

    fn push(&mut self, words: Vec<Word>, span: Span, conditional: bool, heredoc: Option<Word>) {
        self.out.push(Item::Command(Simple {
            words,
            raw: self.slice(span),
            conditional,
            piped: self.piped && heredoc.is_none(),
            heredoc,
        }));
    }

    fn assignment(&mut self, target: &VarRef, value: &AssignmentValue) -> Word {
        let key = target.name.as_ref();
        let subscript = self.reference(target);
        match value {
            AssignmentValue::Scalar(w) => {
                let w = self.word(w);
                Word {
                    text: format!("{key}={}", w.text),
                    unresolved: w.unresolved || subscript,
                }
            }
            AssignmentValue::Compound(arr) => {
                for elem in &arr.elements {
                    match elem {
                        ArrayElem::Sequential(v) => {
                            self.word(&v.word);
                        }
                        ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                            if let Some(w) = &key.word_ast {
                                self.arith_word(w);
                            }
                            self.word(&value.word);
                        }
                    }
                }
                Word {
                    text: format!("{key}={}", self.slice(arr.span)),
                    unresolved: true,
                }
            }
        }
    }

    fn words(&mut self, words: &[AstWord]) {
        for w in words {
            self.word(w);
        }
    }

    // A substitution inside a word is a subshell that runs before the command does. The
    // static text is the command-name decoding, which is the shell's rule for every word.
    fn word(&mut self, w: &AstWord) -> Word {
        for (part, span) in w.parts_with_spans() {
            self.part(part, span);
        }
        match static_command_name_text(w, self.source) {
            Some(text) if !w.has_active_brace_expansion() => Word {
                text: text.into_owned(),
                unresolved: false,
            },
            _ => Word {
                text: w.render(self.source),
                unresolved: true,
            },
        }
    }

    fn part(&mut self, part: &WordPart, span: Span) {
        match part {
            WordPart::CommandSubstitution { body, syntax } => {
                self.substitution(body, *syntax, span);
            }
            WordPart::ProcessSubstitution { body, .. } => self.child(|w| w.seq(body, false)),
            WordPart::DoubleQuoted { parts, .. } => {
                for p in parts {
                    self.part(&p.kind, p.span);
                }
            }
            WordPart::ArithmeticExpansion {
                expression_word_ast,
                ..
            } => {
                self.arith_word(expression_word_ast);
            }
            WordPart::Parameter(p) => self.parameter(p),
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.reference(reference);
                self.op(operator);
                self.optional(operand_word_ast.as_deref());
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.reference(reference);
                if let Some(o) = operator {
                    self.op(o);
                }
                self.optional(operand_word_ast.as_deref());
            }
            WordPart::Length(r)
            | WordPart::ArrayAccess(r)
            | WordPart::ArrayLength(r)
            | WordPart::ArrayIndices(r) => {
                self.reference(r);
            }
            WordPart::Substring {
                reference,
                offset_word_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset_word_ast,
                length_word_ast,
                ..
            } => {
                self.reference(reference);
                self.arith_word(offset_word_ast);
                if let Some(l) = length_word_ast {
                    self.arith_word(l);
                }
            }
            _ => {}
        }
    }

    fn parameter(&mut self, p: &ParameterExpansion) {
        let ParameterExpansionSyntax::Bourne(b) = &p.syntax else {
            return;
        };
        match b {
            BourneParameterExpansion::Access { reference } => {
                self.reference(reference);
                self.prompt_expansion |= reference.name.as_ref().ends_with("@P");
            }
            BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference } => {
                self.reference(reference);
            }
            BourneParameterExpansion::Transformation {
                reference,
                operator,
            } => {
                self.reference(reference);
                self.prompt_expansion |= *operator == 'P';
            }
            BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.reference(reference);
                if let Some(o) = operator {
                    self.op(o);
                }
                self.optional(operand_word_ast.as_deref());
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.reference(reference);
                self.op(operator);
                self.prompt_expansion |= reference.name.as_ref() == "PS4"
                    && matches!(**operator, ParameterOp::AssignDefault)
                    && operand_word_ast
                        .as_deref()
                        .is_some_and(|w| self.quoted_substitution(w));
                self.optional(operand_word_ast.as_deref());
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_word_ast,
                length_word_ast,
                ..
            } => {
                self.reference(reference);
                self.arith_word(offset_word_ast);
                if let Some(l) = length_word_ast {
                    self.arith_word(l);
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        }
    }

    // `${a//x/$(cmd)}`, `${a#$(cmd)}`: the pattern and the replacement run.
    fn op(&mut self, o: &ParameterOp) {
        match o {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => self.pattern(pattern),
            ParameterOp::ReplaceFirst {
                pattern,
                replacement_word_ast,
                ..
            }
            | ParameterOp::ReplaceAll {
                pattern,
                replacement_word_ast,
                ..
            } => {
                self.pattern(pattern);
                self.word(replacement_word_ast);
            }
            _ => {}
        }
    }

    // A subscript runs too: `a[$(cmd)]=1`, `${a[$(cmd)]}`. True when it holds an expansion.
    fn reference(&mut self, r: &VarRef) -> bool {
        let Some(sub) = r.subscript.as_deref() else {
            return false;
        };
        match (sub.word_ast(), sub.arithmetic_ast.as_ref()) {
            (Some(w), _) => self.arith_word(w).unresolved,
            (None, Some(a)) => {
                self.arith(Some(a));
                true
            }
            (None, None) => false,
        }
    }

    // A word read as arithmetic is expanded again, quotes or not.
    fn arith_word(&mut self, w: &AstWord) -> Word {
        self.quoted_arith |= self.quoted_substitution(w);
        self.word(w)
    }

    // Text now, a substitution when bash expands the word again. Judged on the word's static
    // text joined and decoded, with every expansion a placeholder, so `$''(`, `\\$\\(` and
    // `$'\\x24'"("` all show the `$(` they spell.
    fn quoted_substitution(&self, w: &AstWord) -> bool {
        let mut text = String::new();
        for (p, span) in w.parts_with_spans() {
            self.static_part(p, span, &mut text);
        }
        spells_a_substitution(&text)
    }

    fn static_part(&self, p: &WordPart, span: Span, out: &mut String) {
        match p {
            WordPart::Literal(_) => out.push_str(&unescape(&self.slice(span))),
            WordPart::SingleQuoted { value, dollar } => {
                let inner = value.slice(self.source);
                if *dollar {
                    out.push_str(&ansi_decode(inner));
                } else {
                    out.push_str(inner);
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                for n in parts {
                    self.static_part(&n.kind, n.span, out);
                }
            }
            _ => out.push('\u{1}'),
        }
    }

    fn optional(&mut self, w: Option<&AstWord>) {
        if let Some(w) = w {
            self.word(w);
        }
    }

    // The heredoc or here-string on stdin is the only redirect this reader keeps; a
    // substitution in any target or body still runs.
    fn redirects(&mut self, redirects: &[Redirect]) -> Option<Word> {
        let mut stdin = None;
        for r in redirects {
            let on_stdin = r.fd.is_none_or(|fd| fd == 0);
            match &r.target {
                RedirectTarget::Word(w) => {
                    let w = self.word(w);
                    if on_stdin {
                        stdin = (r.kind == RedirectKind::HereString).then_some(w);
                    }
                }
                RedirectTarget::Heredoc(h) => {
                    let mut unresolved = false;
                    for (part, span) in h.body.parts_with_spans() {
                        match part {
                            HeredocBodyPart::Literal(_) => {}
                            HeredocBodyPart::CommandSubstitution { body, syntax } => {
                                unresolved = true;
                                self.substitution(body, *syntax, span);
                            }
                            HeredocBodyPart::ArithmeticExpansion {
                                expression_word_ast,
                                ..
                            } => {
                                unresolved = true;
                                self.arith_word(expression_word_ast);
                            }
                            HeredocBodyPart::Parameter(p) => {
                                unresolved = true;
                                self.parameter(p);
                            }
                            HeredocBodyPart::Variable(_) => unresolved = true,
                        }
                    }
                    if on_stdin {
                        stdin = Some(Word {
                            text: h.body.render(self.source),
                            unresolved,
                        });
                    }
                }
            }
        }
        stdin
    }

    fn slice(&self, span: Span) -> String {
        self.source
            .get(span.start.offset()..span.end.offset())
            .unwrap_or("")
            .trim()
            .trim_end_matches(['&', '|', ';'])
            .trim()
            .to_string()
    }
}
