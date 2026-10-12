<script lang="ts">
  type InstrumentSkin = "phosphor" | "ion" | "breach";
  let { skin = null }: { skin?: InstrumentSkin | null } = $props();
</script>

{#if skin === "phosphor"}
  <svg class="hud-art phosphor" viewBox="0 0 520 280" fill="none" aria-hidden="true" focusable="false">
    <defs>
      <pattern id="hud-blocks" width="18" height="18" patternUnits="userSpaceOnUse">
        <rect width="15" height="15" fill="none" stroke="var(--signal)" stroke-opacity=".55" stroke-width="1" />
      </pattern>
    </defs>
    <rect class="hud-orbit" x="8" y="8" width="504" height="264" />
    <g class="hud-brackets" stroke-width="2.4">
      <path d="M22 48 V22 H48" /><path d="M168 22 H194 V48" /><path d="M194 232 V258 H168" /><path d="M48 258 H22 V232" />
      <path d="M326 48 V22 H352" /><path d="M472 22 H498 V48" /><path d="M498 232 V258 H472" /><path d="M352 258 H326 V232" />
    </g>
    <text class="hud-label" x="36" y="58">SOURCE</text>
    <rect x="36" y="74" width="144" height="108" fill="url(#hud-blocks)" />
    <rect class="hud-scan" x="36" y="74" width="144" height="3" />
    <rect class="hud-cell" x="54" y="92" width="15" height="15" />
    <rect class="hud-cell late" x="108" y="110" width="15" height="15" />
    <rect class="hud-cell later" x="72" y="146" width="15" height="15" />
    <text class="hud-meta" x="36" y="206">3840×2160 · 10-BIT</text>
    <text class="hud-meta" x="36" y="226">READ ONLY</text>
    <rect x="228" y="92" width="64" height="26" stroke="var(--signal)" stroke-width="1.6" />
    <text class="hud-label" x="260" y="110" text-anchor="middle">X265</text>
    <path class="hud-stream" d="M206 156 H314" />
    <circle class="hud-packet" r="4" />
    <circle class="hud-packet delay" r="3" />
    <text class="hud-label" x="340" y="58">VERIFIED</text>
    <circle cx="412" cy="132" r="40" stroke="var(--signal)" stroke-opacity=".28" />
    <circle class="hud-ring" cx="412" cy="132" r="40" />
    <path class="hud-check" d="M394 132 l12 13 26-28" />
    <text class="hud-crc hud-crc-a" x="412" y="202" text-anchor="middle">A3F09C1E</text>
    <text class="hud-crc hud-crc-b" x="412" y="202" text-anchor="middle">6B21DE04</text>
    <text class="hud-meta" x="412" y="226" text-anchor="middle">AUDIO · SUBS · CHAPTERS</text>
  </svg>
{:else if skin === "ion"}
  <svg class="hud-art ion" viewBox="0 0 520 280" fill="none" aria-hidden="true" focusable="false">
    <circle class="ion-ring" cx="260" cy="140" r="118" />
    <circle class="ion-ring mid" cx="260" cy="140" r="78" />
    <circle class="ion-ring inner" cx="260" cy="140" r="38" />
    <path class="ion-cross" d="M260 34 V246 M142 140 H378" />
    <path class="ion-sweep" d="M260 140 L260 34 A106 106 0 0 1 348 78 Z" />
    <polygon class="ion-node" points="72,88 108,66 144,88 144,132 108,154 72,132" />
    <text class="hud-label" x="108" y="58" text-anchor="middle">SOURCE</text>
    <text class="hud-meta" x="108" y="178" text-anchor="middle">CONTACT</text>
    <polygon class="ion-beam" points="150,140 214,128 214,152" />
    <polygon class="ion-beam late" points="306,140 370,128 370,152" />
    <rect class="ion-chip" x="228" y="118" width="64" height="44" />
    <text class="hud-label" x="260" y="145" text-anchor="middle">X265</text>
    <polygon class="ion-node verified" points="376,88 412,66 448,88 448,132 412,154 376,132" />
    <path class="hud-check ion-check" d="M396 110 l10 11 22-24" />
    <text class="hud-label" x="412" y="58" text-anchor="middle">LOCK</text>
    <text class="hud-meta" x="412" y="178" text-anchor="middle">CRC OK</text>
    <text class="hud-meta" x="260" y="248" text-anchor="middle">VECTOR PASS · STREAMS PRESERVED</text>
  </svg>
{:else if skin === "breach"}
  <svg class="hud-art breach" viewBox="0 0 520 280" fill="none" aria-hidden="true" focusable="false">
    <rect class="breach-frame offset" x="24" y="28" width="180" height="150" />
    <rect class="breach-frame" x="18" y="22" width="180" height="150" />
    <text class="hud-label glitch" x="30" y="48">SOURCE</text>
    <path class="breach-slash" d="M34 70 H178 M34 88 H150 M34 106 H166 M34 124 H140" />
    <text class="hud-meta" x="30" y="156">UNTRUSTED INPUT</text>
    <path class="breach-wire" d="M204 96 H248 L268 116 H316" />
    <path class="breach-wire ghost" d="M206 102 H250 L270 122 H318" />
    <polygon class="breach-chip" points="248,74 304,74 316,92 304,110 248,110 236,92" />
    <text class="hud-label" x="276" y="96" text-anchor="middle">X265</text>
    <rect class="breach-frame offset" x="330" y="34" width="168" height="150" />
    <rect class="breach-frame" x="324" y="28" width="168" height="150" />
    <text class="hud-label glitch" x="336" y="54">VERIFIED</text>
    <path class="hud-check breach-check" d="M372 96 l14 16 34-36" />
    <path class="breach-crack" d="M360 130 H460 M380 148 H440" />
    <text class="hud-crc hud-crc-a" x="408" y="168" text-anchor="middle">A3F09C1E</text>
    <text class="hud-crc hud-crc-b glitch" x="410" y="170" text-anchor="middle">!!FAILSAFE</text>
    <text class="hud-meta" x="260" y="232" text-anchor="middle">ORIGINALS HOLD · COMMIT AFTER CHECK</text>
    <path class="breach-hazard" d="M40 214 H480" />
  </svg>
{:else}
  <svg class="workflow-art" viewBox="0 0 520 280" fill="none" aria-hidden="true" focusable="false">
    <defs>
      <linearGradient id="workflow-card" x1="62" y1="42" x2="450" y2="250" gradientUnits="userSpaceOnUse">
        <stop stop-color="var(--workflow-art-start)" />
        <stop offset="1" stop-color="var(--workflow-art-end)" />
      </linearGradient>
      <linearGradient id="workflow-frame" x1="91" y1="91" x2="184" y2="181" gradientUnits="userSpaceOnUse">
        <stop stop-color="var(--workflow-art-frame-start)" />
        <stop offset="1" stop-color="var(--workflow-art-frame-end)" />
      </linearGradient>
      <filter id="workflow-shadow" x="37" y="28" width="450" height="240" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse">
        <feGaussianBlur stdDeviation="12" />
      </filter>
    </defs>
    <circle cx="260" cy="140" r="112" fill="var(--workflow-art-glow)" opacity=".24" filter="url(#workflow-shadow)" />
    <path d="M24 229h472M44 45h432" stroke="var(--workflow-art-grid)" stroke-dasharray="2 9" opacity=".45" />
    <path d="M207 139h96" stroke="var(--workflow-art-path)" stroke-width="2" stroke-dasharray="5 7" />
    <circle cx="249" cy="139" r="4" fill="var(--workflow-art-path)" />
    <circle cx="274" cy="139" r="4" fill="var(--workflow-art-path)" />
    <g filter="url(#workflow-shadow)" opacity=".23">
      <rect x="62" y="52" width="165" height="177" rx="22" fill="#020b07" />
      <rect x="314" y="53" width="142" height="177" rx="22" fill="#020b07" />
    </g>
    <g>
      <rect x="66" y="42" width="165" height="177" rx="21" fill="url(#workflow-card)" stroke="var(--workflow-art-outline)" />
      <rect x="84" y="84" width="128" height="83" rx="13" fill="url(#workflow-frame)" />
      <rect x="94" y="94" width="17" height="63" rx="5" fill="var(--workflow-art-sprocket)" opacity=".84" />
      <rect x="98" y="99" width="8" height="8" rx="2" fill="var(--workflow-art-hole)" />
      <rect x="98" y="114" width="8" height="8" rx="2" fill="var(--workflow-art-hole)" />
      <rect x="98" y="129" width="8" height="8" rx="2" fill="var(--workflow-art-hole)" />
      <rect x="98" y="144" width="8" height="8" rx="2" fill="var(--workflow-art-hole)" />
      <rect x="119" y="98" width="78" height="56" rx="8" fill="var(--workflow-art-frame)" opacity=".8" />
      <path d="M128 139l15-18 11 11 12-15 22 22h-60Z" fill="var(--workflow-art-land)" opacity=".84" />
      <circle cx="178" cy="110" r="5" fill="var(--workflow-art-sun)" />
      <circle cx="198" cy="197" r="5" fill="var(--workflow-art-check)" opacity=".78" />
    </g>
    <path d="M239 139h61" stroke="var(--workflow-art-accent)" stroke-width="2" stroke-linecap="round" />
    <path d="m291 130 10 9-10 9" stroke="var(--workflow-art-accent)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
    <rect x="242" y="102" width="52" height="22" rx="11" fill="var(--workflow-art-pill)" stroke="var(--workflow-art-outline)" />
    <circle cx="268" cy="113" r="3" fill="var(--workflow-art-label)" />
    <g>
      <rect x="314" y="42" width="142" height="177" rx="21" fill="url(#workflow-card)" stroke="var(--workflow-art-outline)" />
      <circle cx="385" cy="123" r="34" fill="var(--workflow-art-badge)" />
      <path d="m369 123 11 11 22-24" stroke="var(--workflow-art-check)" stroke-width="5" stroke-linecap="round" stroke-linejoin="round" />
      <path d="M340 174h90" stroke="var(--workflow-art-outline)" />
      <path d="M357 196h56" stroke="var(--workflow-art-check)" stroke-width="5" stroke-linecap="round" opacity=".58" />
    </g>
  </svg>
{/if}

<style>
  .workflow-art, .hud-art { display: block; width: min(100%, 520px); height: auto; overflow: visible; }
  .hud-art { color: var(--signal); }
  .hud-art text { font-family: "Ubuntu Mono", "DejaVu Sans Mono", ui-monospace, monospace; }
  .hud-label { fill: var(--signal); font-size: 13px; font-weight: 700; letter-spacing: .16em; }
  .hud-meta { fill: var(--signal); fill-opacity: .72; font-size: 10px; letter-spacing: .12em; }

  .phosphor .hud-orbit { stroke: var(--signal); stroke-width: 1.5; stroke-dasharray: 86 1414; animation: hud-orbit 3.6s linear infinite; }
  .phosphor .hud-brackets { stroke: var(--signal); filter: drop-shadow(0 0 4px var(--signal)); }
  .phosphor .hud-scan { fill: var(--signal); transform-box: fill-box; filter: drop-shadow(0 0 6px var(--signal)); animation: hud-scan 2.1s linear infinite; }
  .phosphor .hud-cell { fill: var(--signal); opacity: .15; animation: hud-cell 1.6s ease-in-out infinite; }
  .phosphor .hud-cell.late { animation-delay: .4s; }
  .phosphor .hud-cell.later { animation-delay: .9s; }
  .phosphor .hud-stream { stroke: var(--signal); stroke-width: 2; stroke-dasharray: 7 9; stroke-linecap: square; animation: hud-stream .7s linear infinite; }
  .phosphor .hud-packet { fill: var(--signal); offset-path: path("M206 156 H314"); offset-rotate: 0deg; animation: hud-packet 1.35s linear infinite; }
  .phosphor .hud-packet.delay { animation-delay: .55s; opacity: .7; }
  .phosphor .hud-ring { stroke: var(--signal); stroke-width: 2; stroke-dasharray: 10 8; animation: hud-ring 6s linear infinite; transform-origin: 412px 132px; }
  .phosphor .hud-check { stroke: var(--signal); stroke-width: 4; stroke-linecap: square; stroke-linejoin: miter; stroke-dasharray: 52; stroke-dashoffset: 52; filter: drop-shadow(0 0 5px var(--signal)); animation: hud-check 2.8s ease-in-out infinite; }
  .phosphor .hud-crc { fill: var(--signal); font-size: 13px; font-weight: 700; letter-spacing: .18em; }
  .phosphor .hud-crc-a, .phosphor .hud-crc-b { animation: hud-crc 2.2s step-end infinite; }
  .phosphor .hud-crc-b { animation-delay: 1.1s; }

  .ion text { font-family: "Ubuntu", "DejaVu Sans", system-ui, sans-serif; letter-spacing: .18em; }
  .ion .ion-ring { stroke: var(--signal); stroke-width: 1.2; stroke-opacity: .35; fill: none; }
  .ion .ion-ring.mid { stroke-opacity: .55; stroke-dasharray: 4 10; animation: hud-ring 12s linear infinite; transform-origin: 260px 140px; }
  .ion .ion-ring.inner { stroke-opacity: .85; }
  .ion .ion-cross { stroke: var(--signal); stroke-opacity: .28; stroke-width: 1; }
  .ion .ion-sweep { fill: color-mix(in srgb, var(--signal) 28%, transparent); transform-origin: 260px 140px; animation: hud-ring 3.2s linear infinite; }
  .ion .ion-node { fill: color-mix(in srgb, var(--signal) 12%, transparent); stroke: var(--signal); stroke-width: 1.8; filter: drop-shadow(0 0 6px var(--signal)); }
  .ion .ion-node.verified { fill: color-mix(in srgb, var(--signal) 22%, transparent); }
  .ion .ion-beam { fill: var(--signal); opacity: .55; animation: hud-cell 1.4s ease-in-out infinite; }
  .ion .ion-beam.late { animation-delay: .5s; }
  .ion .ion-chip { fill: #02080e; stroke: var(--signal); stroke-width: 1.6; }
  .ion .ion-check { stroke: var(--signal); stroke-width: 4; stroke-linecap: square; fill: none; filter: drop-shadow(0 0 5px var(--signal)); }

  .breach .breach-frame { stroke: var(--signal); stroke-width: 2; fill: color-mix(in srgb, var(--signal) 6%, transparent); }
  .breach .breach-frame.offset { stroke: #4d1020; transform: translate(5px, 4px); }
  .breach .breach-slash { stroke: var(--signal); stroke-width: 2; stroke-dasharray: 18 8; animation: hud-stream .55s linear infinite; }
  .breach .breach-wire { stroke: var(--signal); stroke-width: 2; fill: none; }
  .breach .breach-wire.ghost { stroke: #00e5ff; stroke-opacity: .55; transform: translate(2px, 2px); }
  .breach .breach-chip { fill: #120306; stroke: var(--signal); stroke-width: 2; filter: drop-shadow(3px 3px 0 #4d1020); }
  .breach .breach-check { stroke: var(--signal); stroke-width: 5; stroke-linecap: square; fill: none; filter: drop-shadow(0 0 6px var(--signal)); animation: hud-check 1.8s ease-in-out infinite; stroke-dasharray: 70; stroke-dashoffset: 70; }
  .breach .breach-crack { stroke: #ffc14d; stroke-width: 2; }
  .breach .breach-hazard { stroke: var(--signal); stroke-width: 3; stroke-dasharray: 10 8; animation: hud-stream .4s linear infinite; }
  .breach .hud-crc { fill: var(--signal); font-size: 13px; font-weight: 700; letter-spacing: .16em; }
  .breach .hud-crc-a, .breach .hud-crc-b { animation: hud-crc 1.4s step-end infinite; }
  .breach .hud-crc-b { animation-delay: .7s; fill: #ffc14d; }
  .breach .glitch { animation: breach-art-glitch 1.6s steps(2, end) infinite; }

  @keyframes hud-orbit { to { stroke-dashoffset: -1500; } }
  @keyframes hud-scan { from { transform: translateY(0); } to { transform: translateY(105px); } }
  @keyframes hud-cell { 0%, 100% { opacity: .12; } 50% { opacity: .9; } }
  @keyframes hud-stream { to { stroke-dashoffset: -32; } }
  @keyframes hud-packet { from { offset-distance: 0%; } to { offset-distance: 100%; } }
  @keyframes hud-ring { to { transform: rotate(360deg); } }
  @keyframes hud-check { 0% { stroke-dashoffset: 70; } 42%, 100% { stroke-dashoffset: 0; } }
  @keyframes hud-crc { 0%, 49% { opacity: 1; } 50%, 100% { opacity: 0; } }
  @keyframes breach-art-glitch {
    0%, 84%, 100% { transform: translate(0); }
    88% { transform: translate(-2px, 1px); }
    92% { transform: translate(2px, -1px); }
  }

  @media (prefers-reduced-motion: reduce) {
    .hud-art :is(.hud-orbit, .hud-scan, .hud-cell, .hud-stream, .hud-packet, .hud-ring, .hud-check, .hud-crc-a, .hud-crc-b, .ion-sweep, .ion-beam, .breach-slash, .breach-hazard, .breach-check, .glitch) { animation: none; }
    .hud-check, .breach-check { stroke-dashoffset: 0; }
    .hud-crc-b { opacity: 0; }
    .hud-cell, .ion-beam { opacity: .55; }
  }
</style>
