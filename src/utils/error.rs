use std::error::Error;

/// Render an error together with every cause beneath it.
///
/// `{}` on an error prints only its own message. That is usually what you want
/// mid-chain, but at the point where an error is finally *reported* it throws
/// away the part that says what actually went wrong.
///
/// A real example this was written for. Indexing a post-Fulu slot reported:
///
/// ```text
/// Error processing slots range 13925134-13925135. Slot 13925134 failed:
/// Failed to index block with root '0xb8019a…' at slot 13925134
/// ```
///
/// which names the block and stops. The cause was three links down and had
/// been captured all along:
///
/// ```text
/// API usage error: Code: 400, Message: "BAD_REQUEST: Insufficient data
/// columns to reconstruct blobs: required 64, but only 0 were found. You may
/// need to run the beacon node with --supernode or --semi-supernode."
/// ```
///
/// The first tells you nothing actionable; the second says the beacon node
/// could not reconstruct the blobs from its PeerDAS data columns, that the
/// slot is fine, and that retrying against another backend will work.
pub fn format_error_chain(err: &dyn Error) -> String {
    let mut out = err.to_string();
    let mut source = err.source();

    // Bounded rather than `while let`: an error chain should never be cyclic,
    // but a formatter that can hang is worse than one that truncates.
    for _ in 0..16 {
        match source {
            Some(cause) => {
                let text = cause.to_string();
                // anyhow repeats its context as a source, so skip a link that
                // adds nothing rather than printing "X: X".
                if !out.ends_with(&text) {
                    out.push_str(&format!(": {text}"));
                }
                source = cause.source();
            }
            None => break,
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] Inner);

    #[derive(Debug, thiserror::Error)]
    #[error("inner cause")]
    struct Inner;

    #[test]
    fn includes_the_source() {
        // The whole point: `{}` alone would print "outer" and lose the reason.
        assert_eq!(Outer(Inner).to_string(), "outer");
        assert_eq!(format_error_chain(&Outer(Inner)), "outer: inner cause");
    }

    #[test]
    fn plain_error_is_unchanged() {
        assert_eq!(format_error_chain(&Inner), "inner cause");
    }

    #[test]
    fn does_not_repeat_a_duplicated_link() {
        // anyhow's context shows up as a source with the same text; printing
        // it twice is noise.
        let err = anyhow::Error::msg("boom");
        assert_eq!(format_error_chain(err.as_ref()), "boom");
    }

    #[test]
    fn unwraps_the_real_indexer_shape() {
        use crate::clients::common::{ClientError, ErrorResponse, NumericOrTextCode};
        use crate::slots_processor::error::{SlotProcessingError, SlotsProcessorError};

        // Exactly what the indexer builds on a PeerDAS 400: the beacon client
        // parses the body into ApiError, index_block() adds the block root via
        // with_context(), and SlotsProcessorError wraps that for the report.
        let api = ClientError::ApiError(ErrorResponse {
            code: NumericOrTextCode::Number(400),
            message: Some(
                "BAD_REQUEST: Insufficient data columns to reconstruct blobs".to_string(),
            ),
        });
        let slot_err: SlotProcessingError = anyhow::Error::new(api)
            .context("Failed to index block with root '0xb8019a' at slot 13925134")
            .into();
        let err = SlotsProcessorError::FailedSlotsProcessing {
            initial_slot: 13925134,
            final_slot: 13925135,
            failed_slot: 13925134,
            error: slot_err,
        };

        let full = format_error_chain(&err);
        assert!(
            full.contains("Insufficient data columns"),
            "the cause is still being dropped: {full}"
        );
    }

    #[test]
    fn unwraps_anyhow_context() {
        // This is the shape the indexer actually produces: index_block returns
        // a client error, .with_context() wraps it with the block root. The
        // outer message alone is the useless one.
        let inner = anyhow::Error::msg(
            "API usage error: Code: 400, Message: \"Insufficient data columns\"",
        );
        let ctx = inner.context("Failed to index block with root '0xabc' at slot 1");

        assert_eq!(
            ctx.to_string(),
            "Failed to index block with root '0xabc' at slot 1"
        );
        let full = format_error_chain(ctx.as_ref());
        assert!(
            full.contains("Insufficient data columns"),
            "cause was lost: {full}"
        );
    }
}
