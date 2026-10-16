//! shuck-ast to `Item`: the only file that names the parser. A parse that is not clean is an
//! error, because a recovered parse means something ran that this reader did not see
//! (DESIGN.md decision 36).

use shuck_ast::{
    AssignmentValue, BinaryOp, Command, CompoundCommand, DeclClause, DeclOperand, HeredocBodyPart,
    Redirect, RedirectTarget, SimpleCommand, Span, Stmt, StmtSeq, Word as AstWord, WordPart,
    static_command_name_text,
};
use shuck_parser::parser::{ParseStatus, Parser};

use super::{Item, Simple, Word};

/// The script's commands in run order, or the parser's first diagnostic.
pub(crate) fn read(script: &str) -> Result<Vec<Item>, String> {
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
    };
    walk.seq(&parsed.file.body, false);
    Ok(walk.out)
}

struct Walk<'a> {
    source: &'a str,
    out: Vec<Item>,
}

impl Walk<'_> {
    fn seq(&mut self, seq: &StmtSeq, conditional: bool) {
        for stmt in &seq.stmts {
            self.stmt(stmt, conditional);
        }
    }

    fn stmt(&mut self, stmt: &Stmt, conditional: bool) {
        let heredoc = self.redirects(&stmt.redirects);
        match &stmt.command {
            Command::Simple(c) => self.simple(c, stmt.span, conditional, heredoc),
            Command::Decl(d) => self.decl(d, stmt.span, conditional),
            Command::Builtin(_) => {}
            Command::Binary(b) => {
                self.stmt(&b.left, conditional);
                self.stmt(&b.right, conditional || b.op == BinaryOp::Or);
            }
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
            CompoundCommand::ArithmeticFor(a) => self.seq(&a.body, true),
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
                    self.seq(&item.body, true);
                }
            }
            CompoundCommand::Select(s) => {
                self.words(&s.words);
                self.seq(&s.body, true);
            }
            CompoundCommand::Subshell(seq) => self.subshell(seq, conditional),
            CompoundCommand::BraceGroup(seq) => self.seq(seq, conditional),
            CompoundCommand::Time(t) => {
                if let Some(cmd) = &t.command {
                    self.stmt(cmd, conditional);
                }
            }
            CompoundCommand::Coproc(c) => {
                let mut child = Walk {
                    source: self.source,
                    out: Vec::new(),
                };
                child.stmt(&c.body, conditional);
                self.out.push(Item::Subshell(child.out));
            }
            CompoundCommand::Always(a) => {
                self.seq(&a.body, conditional);
                self.seq(&a.always_body, true);
            }
            CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => {}
        }
    }

    fn subshell(&mut self, seq: &StmtSeq, conditional: bool) {
        let mut child = Walk {
            source: self.source,
            out: Vec::new(),
        };
        child.seq(seq, conditional);
        self.out.push(Item::Subshell(child.out));
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
        self.out.push(Item::Command(Simple {
            words,
            raw: self.slice(span),
            conditional,
            heredoc,
        }));
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
        self.out.push(Item::Command(Simple {
            words,
            raw: self.slice(span),
            conditional,
            heredoc: None,
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
            AssignmentValue::Compound(arr) => Word {
                text: format!("{key}={}", self.slice(arr.span)),
                unresolved: true,
            },
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
            Some(text) => Word {
                text: text.into_owned(),
                unresolved: false,
            },
            None => Word {
                text: w.render(self.source),
                unresolved: true,
            },
        }
    }

    fn part(&mut self, part: &WordPart) {
        match part {
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => self.subshell(body, false),
            WordPart::DoubleQuoted { parts, .. } => {
                for p in parts {
                    self.part(&p.kind);
                }
            }
            _ => {}
        }
    }

    // The heredoc on stdin is the only redirect this reader keeps; a substitution in any
    // target or body still runs.
    fn redirects(&mut self, redirects: &[Redirect]) -> Option<Word> {
        let mut stdin = None;
        for r in redirects {
            match &r.target {
                RedirectTarget::Word(w) => {
                    self.word(w);
                }
                RedirectTarget::Heredoc(h) => {
                    let mut unresolved = false;
                    for (part, _) in h.body.parts_with_spans() {
                        match part {
                            HeredocBodyPart::Literal(_) => {}
                            HeredocBodyPart::CommandSubstitution { body, .. } => {
                                unresolved = true;
                                self.subshell(body, false);
                            }
                            _ => unresolved = true,
                        }
                    }
                    if r.fd.is_none_or(|fd| fd == 0) {
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
