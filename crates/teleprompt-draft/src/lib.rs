//! Drafting a script from something you have: a document, a transcript,
//! a recording, or a session recorded while talking (`import`), recorded
//! here first (`record`). What a draft is made of is [`derive`], which is
//! pure; the rest reads files, runs tools and listens.

pub mod derive;
pub mod document;
pub mod import;
pub mod listening;
pub mod record;
