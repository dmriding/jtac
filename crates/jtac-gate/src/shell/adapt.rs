//! shuck-ast to `Item`: the only file that names the parser. A parse that is not clean is an
//! error, because a recovered parse means something ran that this reader did not see
//! (DESIGN.md decision 36). Every place the grammar can hold a substitution is visited, since
//! each one runs; a pipe's sides and a backgrounded command are subshells, as in bash.

use shuck_ast::{
    ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArrayElem, AssignmentValue, BinaryOp,
    BourneParameterExpansion, BuiltinCommand, Command, CommandSubstitutionSyntax, CompoundCommand,
    ConditionalExpr, DeclClause, DeclOperand, HeredocBodyPart, ParameterExpansion,
    ParameterExpansionSyntax, Pattern, PatternPart, Redirect, RedirectKind, RedirectTarget,
    SimpleCommand, Span, Stmt, StmtSeq, StmtTerminator, Word as AstWord, WordPart,
    static_command_name_text,
};
use shuck_parser::parser::{ParseStatus, Parser};

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
    Ok(walk.out)
}

struct Walk<'a> {
    source: &'a str,
    out: Vec<Item>,
    depth: usize,
    too_deep: bool,
    /// shuck keeps `\`` in a backtick body as text; bash opens a substitution there.
    escaped_backtick: bool,
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
                self.word(w);
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
            piped: self.piped,
        };
        walk(&mut child);
        self.too_deep |= child.too_deep;
        self.escaped_backtick |= child.escaped_backtick;
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
            self.assignment(a.target.name.as_ref(), &a.value);
        }
    }

    fn conditional(&mut self, e: &ConditionalExpr) {
        match e {
            ConditionalExpr::Binary(b) => {
                self.conditional(&b.left);
                self.conditional(&b.right);
            }
            ConditionalExpr::Unary(u) => self.conditional(&u.expr),
            ConditionalExpr::Parenthesized(p) => self.conditional(&p.expr),
            ConditionalExpr::Word(w) | ConditionalExpr::Regex(w) => {
                self.word(w);
            }
            ConditionalExpr::Pattern(p) => self.pattern(p),
            ConditionalExpr::VarRef(_) => {}
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
            words.push(self.assignment(a.target.name.as_ref(), &a.value));
        }
        let name = self.word(&c.name);
        if !name.text.is_empty() || name.unresolved {
            words.push(name);
        }
        words.extend(c.args.iter().map(|w| self.word(w)));
        self.push(words, span, conditional, heredoc);
    }

    // `export K=v` is read as the command `export` with a `K=v` word, so the git scan sees it.
    fn decl(&mut self, d: &DeclClause, span: Span, conditional: bool) {
        let mut words = vec![Word {
            text: d.variant.as_ref().to_string(),
            unresolved: false,
        }];
        for op in &d.operands {
            words.push(match op {
                DeclOperand::Flag(w) | DeclOperand::Dynamic(w) => self.word(w),
                DeclOperand::Name(v) => Word {
                    text: v.name.as_ref().to_string(),
                    unresolved: false,
                },
                DeclOperand::Assignment(a) => self.assignment(a.target.name.as_ref(), &a.value),
            });
        }
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

    fn assignment(&mut self, key: &str, value: &AssignmentValue) -> Word {
        match value {
            AssignmentValue::Scalar(w) => {
                let w = self.word(w);
                Word {
                    text: format!("{key}={}", w.text),
                    unresolved: w.unresolved,
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
                                self.word(w);
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
                self.word(expression_word_ast);
            }
            WordPart::Parameter(p) => self.parameter(p),
            WordPart::ParameterExpansion {
                operand_word_ast, ..
            }
            | WordPart::IndirectExpansion {
                operand_word_ast, ..
            } => self.optional(operand_word_ast.as_deref()),
            WordPart::Substring {
                offset_word_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                offset_word_ast,
                length_word_ast,
                ..
            } => {
                self.word(offset_word_ast);
                self.optional(length_word_ast.as_deref());
            }
            _ => {}
        }
    }

    fn parameter(&mut self, p: &ParameterExpansion) {
        let ParameterExpansionSyntax::Bourne(b) = &p.syntax else {
            return;
        };
        match b {
            BourneParameterExpansion::Indirect {
                operand_word_ast, ..
            }
            | BourneParameterExpansion::Operation {
                operand_word_ast, ..
            } => self.optional(operand_word_ast.as_deref()),
            BourneParameterExpansion::Slice {
                offset_word_ast,
                length_word_ast,
                ..
            } => {
                self.word(offset_word_ast);
                self.optional(length_word_ast.as_deref());
            }
            _ => {}
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
                            _ => unresolved = true,
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
