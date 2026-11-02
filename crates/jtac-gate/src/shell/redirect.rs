//! The redirects of a command. Stdin is kept only when it is one heredoc or here-string on
//! descriptor 0 with nothing else touching that descriptor; a file, a dup, a closed
//! descriptor, a second redirect there or a process substitution is stdin the reader cannot
//! see, and the command is walked as a pipe's right side, so a shell inside fails closed. A
//! substitution in any target runs with the stdin set so far, so it is walked the same way.
//! A heredoc is read only in its plain shape, where the parser and bash agree on its extent.

use shuck_ast::{
    Heredoc, HeredocBodyPart, HeredocBodyPartNode, Redirect, RedirectKind, RedirectTarget,
    Word as AstWord,
};

use super::Word;
use super::adapt::Walk;
use super::hollow::{cooked_heredoc, degraded, heredoc_cut_short, hollow};
use super::join::cut_within;
use super::laid::placed;

impl Walk<'_> {
    // What stdin holds after the redirects run: the one heredoc or here-string on descriptor
    // 0, or nothing. Anything else on 0 sets `piped`. A redirect with no number written is
    // stdin only for the input kinds; `{fd}<` names a variable, never stdin. The parser lexes
    // a digit run that starts a word before `<<`, `<<-` or `<<<` as the word (`git 3<<<x push`)
    // where bash reads a descriptor, so that shape is cut; `python3<<E` names no descriptor.
    pub(super) fn redirects(&mut self, redirects: &[Redirect]) -> Option<Word> {
        use RedirectKind::{DupInput, HereDoc, HereDocStrip, HereString, Input, ReadWrite};
        let mut on_stdin: Vec<Option<Word>> = Vec::new();
        for r in redirects {
            let reads = matches!(
                r.kind,
                Input | ReadWrite | HereDoc | HereDocStrip | HereString | DupInput
            );
            let (start, b) = (r.span.start.offset(), self.source.as_bytes());
            let run = (0..start)
                .rev()
                .take_while(|&i| b[i].is_ascii_digit())
                .count();
            self.cut |= matches!(r.kind, HereDoc | HereDocStrip | HereString)
                && run > 0
                && start
                    .checked_sub(run + 1)
                    .is_none_or(|i| b[i].is_ascii_whitespace() || b";&|()".contains(&b[i]));
            let text = match &r.target {
                RedirectTarget::Word(w) => {
                    let w = self.target(w);
                    (r.kind == HereString).then_some(w)
                }
                RedirectTarget::Heredoc(h) => {
                    let was = self.piped;
                    self.piped |= !on_stdin.is_empty();
                    let w = self.heredoc(h);
                    self.piped = was;
                    Some(w)
                }
            };
            if r.fd_var.is_none() && r.fd.unwrap_or(i32::from(!reads)) == 0 {
                on_stdin.push(text);
            }
        }
        match on_stdin.as_slice() {
            [] => None,
            [Some(w)] => Some(w.clone()),
            _ => {
                self.piped = true;
                None
            }
        }
    }

    fn target(&mut self, w: &AstWord) -> Word {
        let was = std::mem::replace(&mut self.piped, true);
        let word = self.word(w);
        self.piped = was;
        word
    }

    // Plain shape: a bare delimiter word, or one wholly quoted (`'E'`, `"E"`, `\E`), no
    // continuation in it or in a quoted body (bash keeps those), the body closed on an
    // exact delimiter line, parts laid where the source has them. Quoting is read from the
    // delimiter's own source; a body the parser took as quoted under an unquoted delimiter
    // was never parsed, so its text is judged whole. Under `<<-` bash strips a tab after a
    // newline inside an expansion before it parses the expansion; the parser does not.
    fn heredoc(&mut self, h: &Heredoc) -> Word {
        let d = &h.delimiter;
        let (from, to) = (h.body.span.start.offset(), h.body.span.end.offset());
        let raw = self.slice(d.raw.span);
        let quoted = raw.contains(['\'', '"', '\\']);
        let (dfrom, dto) = (d.raw.span.start.offset(), d.raw.span.end.offset());
        self.cut |= !plain_delimiter(&raw) || cut_within(self.cuts, dfrom, dto);
        self.cut |= quoted && cut_within(self.cuts, from, to);
        let body = self.source.get(from..to);
        self.hollow |= !quoted && d.quoted && body.is_none_or(|b| degraded(b, true));
        let nested = self.depth > 0;
        self.cut |= heredoc_cut_short(self.source, from, to, &d.cooked, d.strip_tabs, nested);
        let text = |p: &HeredocBodyPartNode| matches!(p.kind, HeredocBodyPart::Literal(_));
        let parts = h
            .body
            .parts
            .iter()
            .map(|p| (!text(p), (p.span.start.offset(), p.span.end.offset())));
        self.laid |= !placed(parts, from);
        self.seen.extend([(dfrom, dto), (from, to)]);
        let was = std::mem::replace(&mut self.quoted, true);
        let joined = std::mem::replace(&mut self.rejoined, !quoted);
        let mut unresolved = false;
        for (part, span) in h.body.parts_with_spans() {
            let text = self.slice(span);
            match part {
                HeredocBodyPart::Literal(_) if !quoted => self.degraded(span),
                HeredocBodyPart::Literal(_) => {}
                _ if !quoted && d.strip_tabs && text.contains("\n\t") => self.cut = true,
                HeredocBodyPart::CommandSubstitution { body, syntax } => {
                    unresolved = true;
                    self.substitution(body, *syntax, span);
                }
                HeredocBodyPart::ArithmeticExpansion {
                    expression_word_ast,
                    expression_ast,
                    ..
                } => {
                    unresolved = true;
                    self.hollow |= expression_ast.is_none() && hollow(0, 0, &text);
                    self.arith_body(expression_word_ast);
                }
                HeredocBodyPart::Parameter(p) => {
                    unresolved = true;
                    self.parameter(p);
                }
                HeredocBodyPart::Variable(_) => unresolved = true,
            }
        }
        self.quoted = was;
        self.rejoined = joined;
        let text = h.body.render(self.source);
        Word {
            text: if quoted { text } else { cooked_heredoc(&text) },
            unresolved,
        }
    }
}

// `E`, `'E'`, `"E"` or `\E` with `E` a word of letters, digits and underscores.
fn plain_delimiter(raw: &str) -> bool {
    let inner = raw
        .strip_prefix('\\')
        .or_else(|| raw.strip_prefix('\'')?.strip_suffix('\''))
        .or_else(|| raw.strip_prefix('"')?.strip_suffix('"'))
        .unwrap_or(raw);
    !inner.is_empty()
        && inner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::plain_delimiter;

    #[test]
    fn a_heredoc_delimiter_is_plain_or_the_heredoc_is_unreadable() {
        for (raw, want) in [
            ("EOF", true),
            ("E_1", true),
            ("'EOF'", true),
            ("\"EOF\"", true),
            ("\\EOF", true),
            ("E\\$", false),
            ("'E'F", false),
            ("E\"F\"", false),
            ("{E}", false),
            ("é", false),
            ("EOF-", false),
            ("''", false),
            ("", false),
        ] {
            assert_eq!(plain_delimiter(raw), want, "{raw:?}");
        }
    }
}
