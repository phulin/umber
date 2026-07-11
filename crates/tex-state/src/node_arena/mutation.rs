//! Typed child-handle patches applied before compact storage publication.

use super::copy::ChildPatch;
use super::storage::NodeStorage;
use crate::math::MathField;

impl NodeStorage {
    pub(crate) fn apply_child_patch(&mut self, patch: ChildPatch) {
        match patch {
            ChildPatch::Box { row, child } => self.boxes.children[row] = child,
            ChildPatch::Unset { row, child } => self.unsets.children[row] = child,
            ChildPatch::Leader { row, child } => match &mut self.leaders[row].2 {
                crate::node::LeaderPayload::HList(value)
                | crate::node::LeaderPayload::VList(value) => value.children = child,
                crate::node::LeaderPayload::Rule { .. } => {
                    unreachable!("rule leader cannot carry a child patch")
                }
            },
            ChildPatch::Disc { row, children } => {
                self.discs[row].1 = children[0];
                self.discs[row].2 = children[1];
                self.discs[row].3 = children[2];
            }
            ChildPatch::Insertion { row, child } => self.insertions.content[row] = child,
            ChildPatch::Noad { row, children } => {
                replace_math_child(&mut self.noads.nucleus[row], children[0]);
                replace_math_child(&mut self.noads.subscript[row], children[1]);
                replace_math_child(&mut self.noads.superscript[row], children[2]);
            }
            ChildPatch::Fraction { row, children } => {
                self.fractions[row].numerator = children[0];
                self.fractions[row].denominator = children[1];
            }
            ChildPatch::Choice { row, children } => {
                self.choices[row].display = children[0];
                self.choices[row].text = children[1];
                self.choices[row].script = children[2];
                self.choices[row].script_script = children[3];
            }
            ChildPatch::MathList { row, child } => self.math_lists[row].content = child,
            ChildPatch::Adjust { row, child } => self.adjusts[row] = child,
        }
    }
}

fn replace_math_child(field: &mut MathField, child: Option<crate::ids::NodeListId>) {
    match (field, child) {
        (MathField::SubBox(id) | MathField::SubMlist(id), Some(child)) => *id = child,
        (MathField::Empty | MathField::MathChar(_) | MathField::MathTextChar(_), None) => {}
        _ => unreachable!("math child patch shape changed during compact copy"),
    }
}
