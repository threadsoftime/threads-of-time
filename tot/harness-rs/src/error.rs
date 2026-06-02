//! Application-level error helpers for the REST adapter.
//!
//! `AppError` was removed — it was forward-scaffolding that was never wired
//! into any handler.  Every tool-call response is a typed JSON body produced
//! directly by dispatch; infrastructure failures short-circuit with explicit
//! HTTP status codes, not an `IntoResponse` impl on a shared error enum.
//!
//! This module is retained for future error helpers (e.g. typed JSON factories)
//! that the REST adapter may share.
