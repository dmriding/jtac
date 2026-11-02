//! The arithmetic walk of `Walk`: `(( ))`, `$(( ))`, `for (( ))`, a subscript, a slice offset.
//! bash expands an arithmetic body again when it evaluates it, so a quoted substitution runs
//! there, and a `<(` is an operator except inside a subscript, which an associative array
//! expands as a word.

use shuck_ast::{ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, Span, Word as AstWord};

use super::Word;
use super::adapt::Walk;
use super::reread::quoted_substitution;

impl Walk<'_> {
    // `$(…)` is a primary of arithmetic too; every other node only holds more nodes.
    pub(super) fn arith(&mut self, node: Option<&ArithmeticExprNode>) {
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

    // A word read as arithmetic is expanded again, quotes or not.
    pub(super) fn arith_word(&mut self, w: &AstWord) -> Word {
        self.quoted_arith |= quoted_substitution(self.source, w);
        self.word(w)
    }

    // `<(` is an operator in an arithmetic body, text to bash, except in a subscript, which an
    // associative array expands as a word: a `[` anywhere in the body fails closed.
    pub(super) fn arith_in(&mut self, span: Span, f: impl FnOnce(&mut Self)) {
        let text = !self.slice(span).contains('[');
        let was = std::mem::replace(&mut self.quoted, text);
        f(self);
        self.quoted = was;
    }

    pub(super) fn arith_body(&mut self, w: &AstWord) {
        self.arith_in(w.span, |k| {
            k.arith_word(w);
        });
    }
}
