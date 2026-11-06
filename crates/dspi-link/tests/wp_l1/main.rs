//! WP L1: the Link client against a fake hub that behaves like the real ones,
//! one module per finding of LINK-HUB-FINDINGS.md. Liveness runs on
//! `ManualClock`, so no test waits out a real limit.

mod support;

mod f1;
mod f2;
mod f3;
mod hub;
