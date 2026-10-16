//! shuck-ast to `Item`: the only file that names the parser. A parse that is not clean is an
//! error, because a recovered parse means something ran that this reader did not see
//! (DESIGN.md decision 36). Every place the grammar can hold a substitution is visited, since
//! each one runs; a pipe's sides and a backgrounded command are subshells, as in bash.

use shuck_ast::{
    ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArrayElem, AssignmentValue, BinaryOp,
    BourneParameterExpansion, BuiltinCommand, Command, CompoundCommand, ConditionalExpr,
    DeclClause, DeclOperand, HeredocBodyPart, ParameterExpansion, ParameterExpansionSyntax,
    Pattern, PatternPart, Redirect, RedirectKind, RedirectTarget, SimpleCommand, Span, Stmt,
    StmtSeq, StmtTerminator, Word as AstWord, WordPart, static_command_name_text,
};
use shuck_parser::parser::{ParseStatus, Parser};

use super::{Item, Simple, Word};

/// Brackets nested past this are not read: the parser and this walk recurse, and the stack
/// is finite. Counted on the text before the parser runs, quotes ignored, so it over-counts.
const MAX_SUBSHELLS: u32 = 16;

fn nests_too_deep(script: &str) -> bool {
    let mut depth: u32 = 0;
    for c in script.chars() {
        match c {
            '(' | '{' | '`' => {
                depth += 1;
                if depth > MAX_SUBSHELLS {
                    return true;
                }
            }
            ')' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

/// The script's commands in run order, or why they cannot be read.
pub(crate) fn read(script: &str) -> Result<Vec<Item>, String> {
    if nests_too_deep(script) {
        return Err(format!(
            "it nests brackets or substitutions more than {MAX_SUBSHELLS} deep"
        ));
    }
    // shuck keeps `\`` as text, but inside a backtick body it opens a nested substitution.
    if script.contains("\\`") {
        return Err("it escapes a backtick, which may nest a substitution".to_string());
    }
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
        piped_next: false,
    };
    walk.seq(&parsed.file.body, false);
    if walk.too_deep {
        return Err(format!(
            "it nests subshells or substitutions more than {MAX_SUBSHELLS} deep"
        ));
    }
    Ok(walk.out)
}

struct Walk<'a> {
    source: &'a str,
    out: Vec<Item>,
    depth: u32,
    too_deep: bool,
    piped_next: bool,
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
                    self.child(|w| w.stmt(&b.left, conditional));
                    self.child(|w| {
                        w.piped_next = true;
                        w.stmt(&b.right, conditional);
                    });
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
            piped_next: false,
        };
        walk(&mut child);
        self.too_deep |= child.too_deep;
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
            heredoc,
            piped: std::mem::take(&mut self.piped_next),
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
        for (part, _) in w.parts_with_spans() {
            self.part(part);
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

    fn part(&mut self, part: &WordPart) {
        match part {
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => self.child(|w| w.seq(body, false)),
            WordPart::DoubleQuoted { parts, .. } => {
                for p in parts {
                    self.part(&p.kind);
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
                    if on_stdin && r.kind == RedirectKind::HereString {
                        stdin = Some(w);
                    }
                }
                RedirectTarget::Heredoc(h) => {
                    let mut unresolved = false;
                    for (part, _) in h.body.parts_with_spans() {
                        match part {
                            HeredocBodyPart::Literal(_) => {}
                            HeredocBodyPart::CommandSubstitution { body, .. } => {
                                unresolved = true;
                                self.child(|w| w.seq(body, false));
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
