import { appendFileSync } from "node:fs";

/** Opt-in traces use only the synthetic arguments supplied by test fixtures. */
export function testTrace(suite: string) {
    let sequence = 0;
    return (event: string, data: unknown = null) => {
        if (!process.env.TAURSCRIBE_TEST_TRACE) return;
        appendFileSync(process.env.TAURSCRIBE_TEST_TRACE,
            JSON.stringify({ suite, sequence: ++sequence, at: new Date().toISOString(), event, data }) + "\n");
    };
}
