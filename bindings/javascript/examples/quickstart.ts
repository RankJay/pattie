// "Agent Mic": a virtual microphone any app can pick, playing a 440 Hz tone.
//
//     npm install pattie
//     node quickstart.ts      # Node 24 or newer: `using` and TypeScript run as they are
//
// Pick "Agent Mic" in Zoom, Teams or Meet; Ctrl+C removes it.

import { Microphone } from 'pattie';

using mic = new Microphone('Agent Mic', { format: { rate: 24_000, encoding: 'pcm16' } });
const tone = Int16Array.from({ length: 24_000 }, (_, i) =>
    Math.round(8_000 * Math.sin((2 * Math.PI * 440 * i) / 24_000)),
);
for (;;) await mic.write(tone); // one second at a time, at the pace listeners take it
