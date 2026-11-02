//! shuck-ast to `Item`: the only file that names the parser. A parse that is not clean is an
//! error, because a recovered parse means something ran that this reader did not see
//! (DESIGN.md decision 36). Every place the grammar can hold a substitution is visited, since
//! each one runs; a pipe's sides and a backgrounded command are subshells, as in bash.

use shuck_ast::{
    ArrayElem, AssignmentValue, BinaryOp, BourneParameterExpansion, BuiltinCommand, Command,
    CommandSubstitutionSyntax, CompoundCommand, ConditionalBinaryOp, ConditionalExpr,
    ConditionalUnaryOp, DeclClause, DeclOperand, ParameterExpansion, ParameterExpansionSyntax,
    ParameterOp, Pattern, PatternPart, PatternPartNode, SimpleCommand, Span, Stmt, StmtSeq,
    StmtTerminator, VarRef, Word as AstWord, WordPart, WordPartNode, static_command_name_text,
};
use shuck_parser::parser::{ParseStatus, Parser};

use super::comment::commented;
use super::join::{cut_within, join_continuations, original_span};
use super::laid::{framed, placed, runs};
use super::reread::{evaluates, quoted_substitution, slice, static_text};
use super::scan::{MAX_SUBSHELLS, too_deep_or_long};

use super::{Item, Simple, Word};

/// The parser recurses once per nested construct, its stack budget does not meter every
/// recursion, and the hook's stack is not ours to size. `time` repeated to the size cap takes
/// about 7 KiB of stack per byte in a debug build, 440 MiB; this is the margin, virtual and
/// untouched until needed; `every_shape_under_the_size_cap_parses_without_aborting` aborts past it.
const PARSER_STACK: usize = 512 << 20;

/// The script's commands in run order, or why they cannot be read. A parser panic or a
/// thread the system refuses is unreadable, never an abort of the caller.
pub(crate) fn read(script: &str) -> Result<Vec<Item>, String> {
    if let Some(why) = too_deep_or_long(script) {
        return Err(why);
    }
    let original = script.to_string();
    std::thread::Builder::new()
        .name("jtac-shell-parser".to_string())
        .stack_size(PARSER_STACK)
        .spawn(move || {
            let (joined, cuts) = join_continuations(&original);
            parse(&joined, &cuts, &original)
        })
        .map_err(|e| format!("the parser could not start a thread: {e}"))?
        .join()
        .map_err(|_| "the parser panicked".to_string())?
}

// `script` is `original` with its line continuations joined; `cuts` says where.
fn parse(script: &str, cuts: &[usize], original: &str) -> Result<Vec<Item>, String> {
    let parsed = Parser::new(script).parse();
    if parsed.status != ParseStatus::Clean {
        return Err(parsed.diagnostics.first().map_or_else(
            || "the parser stopped without a diagnostic".to_string(),
            |d| d.message.clone(),
        ));
    }
    let mut walk = Walk {
        source: script,
        cuts,
        original,
        ..Walk::default()
    };
    walk.seq(&parsed.file.body, false);
    if walk.too_deep {
        return Err(format!(
            "it nests subshells or substitutions more than {MAX_SUBSHELLS} deep"
        ));
    }
    if walk.hollow {
        return Err(
            "it holds a substitution, operand or arithmetic the parser read as nothing".to_string(),
        );
    }
    if walk.escaped_backtick {
        return Err("it escapes a backtick, dollar or backslash in a backtick body".to_string());
    }
    if walk.laid {
        return Err(
            "it escapes a quote, dollar or backslash inside a `${…}` operand or before a substitution in the same `\"…\"`, and the parser lays what follows off the source"
                .to_string(),
        );
    }
    if commented(script, cuts, &walk.seen, &walk.open) {
        return Err("a line continuation ends a comment, and bash runs the next line".to_string());
    }
    if walk.cut {
        return Err("it reads a heredoc or here-string other than bash does".to_string());
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

#[derive(Default)]
pub(super) struct Walk<'a> {
    /// The script with its line continuations joined, which every span indexes.
    pub(super) source: &'a str,
    /// Where a continuation was joined (`join_continuations`).
    pub(super) cuts: &'a [usize],
    /// The script as written, for the text of a `'…'` that holds a continuation.
    original: &'a str,
    /// Inside an unquoted heredoc body or a backtick body, which bash joins before it reads
    /// any quote, so a `'…'` there is joined too.
    pub(super) rejoined: bool,
    out: Vec<Item>,
    pub(super) depth: usize,
    too_deep: bool,
    /// A body, operand, pattern or expression the parser dropped under a clean status.
    pub(super) hollow: bool,
    /// Where `<(` is text to bash: `"…"`, an arithmetic body without a subscript, a heredoc body.
    pub(super) quoted: bool,
    /// bash cooks `\``, `\$` and `\\` in a backtick body before it parses the body; shuck
    /// keeps the first as text, and after the others its spans no longer match the source.
    escaped_backtick: bool,
    /// A word, `"…"` segment or heredoc body whose parts do not tile its span (`laid::tiles`).
    pub(super) laid: bool,
    /// Every word, pattern, heredoc and backtick body span read, and every `$(…)` or `<(…)`
    /// inside one that bash reads as a script again (`comment::commented`).
    pub(super) seen: Vec<(usize, usize)>,
    open: Vec<(usize, usize)>,
    /// A heredoc or here-string read other than bash reads it (`redirect::heredoc`).
    pub(super) cut: bool,
    /// `(( '$(cmd)' ))`, `a['$(cmd)']`: bash expands an arithmetic body or a subscript again
    /// when it evaluates it, so a quoted substitution there runs.
    pub(super) quoted_arith: bool,
    /// `${x@P}`: bash expands the value as a prompt, substitutions included.
    prompt_expansion: bool,
    /// Stdin may be another command's output or anything else this text does not hold: a
    /// pipe's right side, a redirect target, a process substitution, an unresolved stdin.
    pub(super) piped: bool,
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

    // Stdin handed to a compound command or a function reaches every command inside, as a
    // pipe's does.
    fn stmt_here(&mut self, stmt: &Stmt, conditional: bool) {
        let was = self.piped;
        let heredoc = self.redirects(&stmt.redirects);
        self.piped |= heredoc.is_some();
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
        self.piped = was;
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
                for e in [&a.init_ast, &a.condition_ast, &a.step_ast]
                    .into_iter()
                    .flatten()
                {
                    self.arith_in(e.span, |w| w.arith(Some(e)));
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
            CompoundCommand::Arithmetic(a) => {
                let text = a.expr_span.map_or_else(String::new, |s| self.slice(s));
                self.hollow |= a.expr_ast.is_none() && !text.is_empty();
                match a.expr_span {
                    Some(span) => {
                        self.seen.push((span.start.offset(), span.end.offset()));
                        self.arith_in(span, |w| w.arith(a.expr_ast.as_ref()));
                    }
                    None => self.arith(a.expr_ast.as_ref()),
                }
            }
        }
    }

    pub(super) fn degraded(&mut self, span: Span) {
        let text = self.source.get(span.start.offset()..span.end.offset());
        self.hollow |= text.is_none_or(|t| super::hollow::degraded(t, self.quoted));
    }

    pub(super) fn substitution(
        &mut self,
        body: &StmtSeq,
        syntax: CommandSubstitutionSyntax,
        span: Span,
    ) {
        let (from, to) = (span.start.offset(), span.end.offset());
        let Some(text) = self.source.get(from..to).map(str::trim) else {
            self.hollow = true;
            return;
        };
        self.escaped_backtick |= syntax == CommandSubstitutionSyntax::Backtick
            && ["\\`", "\\$", "\\\\"].iter().any(|e| text.contains(e));
        let comments = body.leading_comments.len() + body.trailing_comments.len();
        self.hollow |= super::hollow::hollow(body.stmts.len(), comments, text);
        let joined = self.rejoined;
        if syntax == CommandSubstitutionSyntax::Backtick {
            self.rejoined = true;
            self.seen.push((from, to));
        } else if !self.rejoined {
            self.open.push((from, to));
        }
        self.child(|w| w.seq(body, false));
        self.rejoined = joined;
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
            cuts: self.cuts,
            original: self.original,
            rejoined: self.rejoined,
            depth: self.depth + 1,
            piped: self.piped,
            ..Walk::default()
        };
        walk(&mut child);
        self.too_deep |= child.too_deep;
        self.hollow |= child.hollow;
        self.escaped_backtick |= child.escaped_backtick;
        self.laid |= child.laid;
        self.seen.extend(child.seen);
        self.open.extend(child.open);
        self.cut |= child.cut;
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
        let text = |n: &PatternPartNode| matches!(n.kind, PatternPart::Literal(_));
        let parts = p
            .parts
            .iter()
            .map(|n| (!text(n), (n.span.start.offset(), n.span.end.offset())));
        self.laid |= !placed(parts, p.span.start.offset());
        self.seen.push((p.span.start.offset(), p.span.end.offset()));
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
                PatternPart::Literal(_) => self.degraded(part.span),
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
        let quoted: Vec<bool> = c
            .args
            .iter()
            .map(|w| quoted_substitution(self.source, w))
            .collect();
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
                DeclOperand::Dynamic(w) => quoted_substitution(self.source, w),
                DeclOperand::Assignment(a) => {
                    matches!(&a.value, AssignmentValue::Scalar(w) if quoted_substitution(self.source, w))
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
    pub(super) fn word(&mut self, w: &AstWord) -> Word {
        self.laid |= !placed(spans(&w.parts), w.span.start.offset());
        self.seen.push((w.span.start.offset(), w.span.end.offset()));
        for (part, span) in w.parts_with_spans() {
            self.part(part, span);
        }
        match static_command_name_text(w, self.source) {
            Some(_) if self.quoted_continuation(w) => Word {
                text: static_text(self.source, w, &|span, dollar| {
                    self.single_quoted(span, dollar)
                }),
                unresolved: false,
            },
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

    // bash keeps `\<newline>` inside `'…'`, so such a word is read from the original text.
    fn quoted_continuation(&self, w: &AstWord) -> bool {
        !self.rejoined
            && !w.has_active_brace_expansion()
            && w.parts.iter().any(|p| {
                matches!(p.kind, WordPart::SingleQuoted { .. })
                    && cut_within(self.cuts, p.span.start.offset(), p.span.end.offset())
            })
    }

    fn single_quoted(&self, span: Span, dollar: bool) -> String {
        let (from, to) = original_span(self.cuts, span.start.offset(), span.end.offset());
        let from = from + 1 + usize::from(dollar);
        self.original
            .get(from..to.saturating_sub(1))
            .unwrap_or("")
            .to_string()
    }

    fn part(&mut self, part: &WordPart, span: Span) {
        let text = self.source.get(span.start.offset()..span.end.offset());
        self.laid |= text.is_none_or(|t| !framed(t, part));
        match part {
            WordPart::CommandSubstitution { body, syntax } => {
                self.substitution(body, *syntax, span);
            }
            // `>(cmd)` reads what is written to it; `<(cmd)` inherits a stdin this text may
            // not hold either.
            WordPart::ProcessSubstitution { body, .. } => {
                let was = std::mem::replace(&mut self.piped, true);
                self.substitution(body, CommandSubstitutionSyntax::DollarParen, span);
                self.piped = was;
            }
            WordPart::DoubleQuoted { parts, dollar } => {
                let was = std::mem::replace(&mut self.quoted, true);
                let from = span.start.offset() + 1 + usize::from(*dollar);
                self.laid |= !placed(spans(parts), from);
                for p in parts {
                    self.part(&p.kind, p.span);
                }
                self.quoted = was;
            }
            WordPart::ArithmeticExpansion {
                expression_word_ast,
                expression_ast,
                ..
            } => {
                let text = self.slice(span);
                self.hollow |= expression_ast.is_none() && super::hollow::hollow(0, 0, &text);
                self.arith_body(expression_word_ast);
            }
            WordPart::Parameter(p) => self.parameter(p),
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.reference(reference);
                self.op(operator, operand_word_ast.as_deref());
            }
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.reference(reference);
                match operator {
                    Some(o) => self.op(o, operand_word_ast.as_deref()),
                    None => self.optional(operand_word_ast.as_deref()),
                }
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
                self.arith_body(offset_word_ast);
                if let Some(l) = length_word_ast {
                    self.arith_body(l);
                }
            }
            WordPart::Literal(_) => self.degraded(span),
            _ => {}
        }
    }

    pub(super) fn parameter(&mut self, p: &ParameterExpansion) {
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
                match operator {
                    Some(o) => self.op(o, operand_word_ast.as_deref()),
                    None => self.optional(operand_word_ast.as_deref()),
                }
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                self.reference(reference);
                self.prompt_expansion |= reference.name.as_ref() == "PS4"
                    && matches!(**operator, ParameterOp::AssignDefault)
                    && operand_word_ast
                        .as_deref()
                        .is_some_and(|w| quoted_substitution(self.source, w));
                self.op(operator, operand_word_ast.as_deref());
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_word_ast,
                length_word_ast,
                ..
            } => {
                self.reference(reference);
                self.arith_body(offset_word_ast);
                if let Some(l) = length_word_ast {
                    self.arith_body(l);
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        }
    }

    // `${a//x/$(cmd)}`, `${a#$(cmd)}`, `${a^$(cmd)}`: a pattern, a replacement or a case
    // operand runs, expanded unquoted even inside `"…"`, where `"${a:-<(cmd)}"` is text.
    fn op(&mut self, o: &ParameterOp, operand: Option<&AstWord>) {
        use ParameterOp::{AssignDefault, Error, UseDefault, UseReplacement};
        let text = matches!(o, UseDefault | AssignDefault | UseReplacement | Error);
        let was = self.quoted;
        self.quoted = was && text;
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
        self.optional(operand);
        self.quoted = was;
    }

    // A subscript runs too: `a[$(cmd)]=1`, `${a[$(cmd)]}`. True when it holds an expansion. An
    // associative array expands its subscript as a word, so `<(` runs there even in `"…"`.
    fn reference(&mut self, r: &VarRef) -> bool {
        let Some(sub) = r.subscript.as_deref() else {
            return false;
        };
        let was = std::mem::replace(&mut self.quoted, false);
        let expands = match (sub.word_ast(), sub.arithmetic_ast.as_ref()) {
            (Some(w), _) => self.arith_word(w).unresolved,
            (None, Some(a)) => {
                self.arith(Some(a));
                true
            }
            (None, None) => false,
        };
        self.quoted = was;
        expands
    }

    fn optional(&mut self, w: Option<&AstWord>) {
        if let Some(w) = w {
            self.word(w);
        }
    }

    pub(super) fn slice(&self, span: Span) -> String {
        slice(self.source, span)
    }
}

fn spans(parts: &[WordPartNode]) -> impl Iterator<Item = (bool, (usize, usize))> + '_ {
    parts
        .iter()
        .map(|p| (runs(&p.kind), (p.span.start.offset(), p.span.end.offset())))
}
