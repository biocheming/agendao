// P1-1 Rendering layer — terminal output contract.
//
// Rendering functions take projection data (transcripts, blocks, labels)
// and produce formatted terminal output. They MUST NOT mutate projection
// state (CliExecutionRuntime, CliFrontendProjection, CliVisibleTranscript).
// They MUST NOT read keyboard input or dispatch events.
//
// Currently, block→terminal-string formatting lives in the rocode-command
// crate (output_blocks.rs). CLI-specific rendering functions (e.g.
// cli_render_session_block) are in session_projection.rs alongside
// projection logic. As the four-layer split progresses, pure rendering
// functions should migrate here.
//
// See run.rs § "P1-1 four-layer CLI architecture" for the full layering
// contract.
