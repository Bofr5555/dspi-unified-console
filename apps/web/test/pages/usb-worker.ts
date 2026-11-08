// WP WB4a: the page's own Worker (src/core/worker.ts, the real core), with the
// WebUSB mock installed first.
import "./usb-worker-mock";
import "../../src/core/worker";
