//! TON cell codecs for W5R1, TEP-74 jetton transfers, and Highload V3.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::needless_question_mark,
    clippy::missing_const_for_fn,
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    clippy::redundant_closure,
    reason = "BoC bit packing matches TEP-74 / W5R1 / Highload layouts"
)]

pub mod common;
pub mod highload_v3;
pub mod jetton;
pub mod w5;
