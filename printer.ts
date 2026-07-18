import QRCode from "qrcode";
import JsBarcode from "jsbarcode";

interface PrintEvent {
  type: "init" | "align" | "style" | "text" | "new_line" | "cut" | "image" | "qr_code" | "barcode" | "pulse";
  value?: "left" | "center" | "right";
  font?: "a" | "b";
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  invert?: boolean;
  width?: number;
  height?: number;
  text?: string;
  data?: string;
  symbology?: string;
  bits?: string;
}

const feedEl = document.getElementById("feed")!;
const statusEl = document.getElementById("status")!;
const statusText = document.getElementById("statusText")!;
const led = document.getElementById("led")!;
const drawer = document.getElementById("drawer")!;
const receiptDesk = document.getElementById("receiptDesk")!;
const soundToggle = document.getElementById("soundToggle") as HTMLButtonElement;
const paperWidthInput = document.getElementById("paperWidth") as HTMLInputElement;
const hint = document.getElementById("hint")!;
const printerSound = document.getElementById("printerSound") as HTMLAudioElement;
const cutterSound = document.getElementById("cutterSound") as HTMLAudioElement;
const cashDrawerSound = document.getElementById("cashDrawerSound") as HTMLAudioElement;

// ---- print state ----

let align: "left" | "center" | "right" = "left";
let font: "a" | "b" = "a";
let bold = false;
let italic = false;
let underline = false;
let invert = false;
let width = 1;
let height = 1;

let currentFeedPaper: HTMLElement | null = null;
let linesContainer: HTMLElement | null = null;
let currentLineEl: HTMLElement | null = null;
let currentRunEl: HTMLElement | null = null;
let currentRunTextEl: HTMLElement | null = null;
let currentRunStyleKey = "";
let topReceiptZ = 100;
let receiptCount = 0;
let composingReceipt = false;

// ---- sound (opt-in so browser autoplay rules are respected) ----

let soundEnabled = true;
let audioContext: AudioContext | null = null;
printerSound.preload = "auto";
printerSound.volume = 0.28;
printerSound.loop = true;
cutterSound.preload = "auto";
cutterSound.volume = 0.34;
cashDrawerSound.preload = "auto";
cashDrawerSound.volume = 0.5;

function getAudioContext(): AudioContext {
  audioContext ??= new AudioContext();
  if (audioContext.state === "suspended") void audioContext.resume();
  return audioContext;
}

function playTone(frequency: number, duration: number, volume: number, type: OscillatorType = "sine") {
  if (!soundEnabled) return;
  const context = getAudioContext();
  const oscillator = context.createOscillator();
  const gain = context.createGain();
  const now = context.currentTime;

  oscillator.type = type;
  oscillator.frequency.setValueAtTime(frequency, now);
  oscillator.frequency.exponentialRampToValueAtTime(Math.max(40, frequency * 0.72), now + duration);
  gain.gain.setValueAtTime(volume, now);
  gain.gain.exponentialRampToValueAtTime(0.0001, now + duration);
  oscillator.connect(gain).connect(context.destination);
  oscillator.start(now);
  oscillator.stop(now + duration);
}

function playPrintSound() {
  if (!soundEnabled || composingReceipt) return;
  if (printerSound.paused) {
    printerSound.currentTime = 0;
    void printerSound.play().catch(() => undefined);
  }
}

function stopPrintSound() {
  printerSound.pause();
  printerSound.currentTime = 0;
}

function playCutterSound() {
  if (!soundEnabled) return;
  stopPrintSound();
  cutterSound.currentTime = 0;
  void cutterSound.play().catch(() => undefined);
}

function stopCutterSound() {
  cutterSound.pause();
  cutterSound.currentTime = 0;
}

function playCashDrawerSound() {
  if (!soundEnabled) return;
  cashDrawerSound.currentTime = 0;
  void cashDrawerSound.play().catch(() => undefined);
}

soundToggle.addEventListener("click", () => {
  soundEnabled = !soundEnabled;
  soundToggle.setAttribute("aria-pressed", String(soundEnabled));
  soundToggle.querySelector("span")!.textContent = soundEnabled ? "Sound on" : "Sound off";
  if (soundEnabled) {
    getAudioContext();
    playTone(520, 0.08, 0.025, "sine");
  } else {
    stopPrintSound();
    stopCutterSound();
    cashDrawerSound.pause();
    cashDrawerSound.currentTime = 0;
  }
});

// ---- paper width ----
//
// The control is in millimetres (the number printed on a roll's packaging),
// but layout needs the character column count ESC/POS actually thinks in, so
// the two standard sizes are pinned to their real column counts and anything
// else is derived.
//
// The derivation can't just scale a single ratio: at 203 dpi (8 dots/mm) with
// a 12-dot font A cell, an 80mm roll prints 72mm / 576 dots / 48 columns, but
// a 58mm roll prints only 48mm / 384 dots / 32 columns. The unprintable
// margin is 8mm on one and 10mm on the other, so a fixed-margin formula gets
// 58mm wrong by a column. Hence the table for the sizes people actually load,
// and a nominal 8mm margin for everything in between.
//
// Receipts already torn off keep the width they were printed at, since
// handleCut freezes their pixel size before moving them to the desk.

const PAPER_MM_MIN = 32;
const PAPER_MM_MAX = 152;
const PAPER_MM_KEY = "escpos-sim:paper-mm";
const PAPER_COLS_MIN = 16;
const PAPER_COLS_MAX = 96;
const PAPER_MARGIN_MM = 8;
const DOTS_PER_MM = 8; // 203 dpi
const DOTS_PER_CHAR = 12; // font A cell

const STANDARD_COLS: Record<number, number> = { 58: 32, 80: 48 };

function colsForMm(mm: number): number {
  const cols = STANDARD_COLS[mm] ?? Math.round(((mm - PAPER_MARGIN_MM) * DOTS_PER_MM) / DOTS_PER_CHAR);
  return Math.min(PAPER_COLS_MAX, Math.max(PAPER_COLS_MIN, cols));
}

function applyPaperMm(mm: number) {
  document.documentElement.style.setProperty("--paper-cols", String(colsForMm(mm)));
  try {
    localStorage.setItem(PAPER_MM_KEY, String(mm));
  } catch {
    // Private-mode storage failures shouldn't break the width change itself.
  }
}

function clampPaperMm(value: number, fallback: number): number {
  if (!Number.isFinite(value)) return fallback;
  return Math.min(PAPER_MM_MAX, Math.max(PAPER_MM_MIN, Math.round(value)));
}

// Number("") and Number(null) are both 0, so an absent entry has to be ruled
// out before parsing or it would clamp to PAPER_MM_MIN instead of falling
// back to the markup default.
const storedMm = (() => {
  try {
    const raw = localStorage.getItem(PAPER_MM_KEY);
    return raw ? Number(raw) : NaN;
  } catch {
    return NaN;
  }
})();

const initialMm = clampPaperMm(storedMm, Number(paperWidthInput.value) || 80);
paperWidthInput.value = String(initialMm);
applyPaperMm(initialMm);

// `input` keeps the paper live while typing; `change`/blur is where a partial
// or out-of-range entry gets normalised back into the field.
paperWidthInput.addEventListener("input", () => {
  const raw = Number(paperWidthInput.value);
  if (!paperWidthInput.value.trim() || !Number.isFinite(raw)) return;
  applyPaperMm(clampPaperMm(raw, initialMm));
});

paperWidthInput.addEventListener("change", () => {
  const mm = clampPaperMm(Number(paperWidthInput.value), initialMm);
  paperWidthInput.value = String(mm);
  applyPaperMm(mm);
});

function newFeedPaper(): HTMLElement {
  const paper = document.createElement("div");
  paper.className = "feed__paper";
  const lines = document.createElement("div");
  lines.className = "feed__lines";
  paper.appendChild(lines);
  feedEl.appendChild(paper);

  linesContainer = lines;
  currentLineEl = null;
  currentRunEl = null;
  currentRunTextEl = null;
  currentRunStyleKey = "";
  return paper;
}

function ensureFeedPaper(): HTMLElement {
  currentFeedPaper ??= newFeedPaper();
  return currentFeedPaper;
}

function resetStyle() {
  align = "left";
  font = "a";
  bold = false;
  italic = false;
  underline = false;
  invert = false;
  width = 1;
  height = 1;
}

function styleKey(): string {
  return `${align}|${font}|${bold}|${italic}|${underline}|${invert}|${width}|${height}`;
}

function ensureLine(): HTMLElement {
  ensureFeedPaper();
  if (!currentLineEl) {
    currentLineEl = document.createElement("div");
    currentLineEl.className = "line";
    currentLineEl.style.textAlign = align;
    linesContainer!.appendChild(currentLineEl);
    currentRunEl = null;
    currentRunTextEl = null;
    currentRunStyleKey = "";
  }
  return currentLineEl;
}

function appendText(text: string) {
  if (!text) return;
  playPrintSound();
  const line = ensureLine();
  const key = styleKey();

  if (!currentRunEl || currentRunStyleKey !== key) {
    currentRunEl = document.createElement("span");
    currentRunEl.className = "run";
    if (font === "b") currentRunEl.classList.add("font-b");
    if (bold) currentRunEl.classList.add("bold");
    if (italic) currentRunEl.classList.add("italic");
    if (underline) currentRunEl.classList.add("underline");
    if (invert) currentRunEl.classList.add("invert");
    if (width > 1 || height > 1) {
      // The outer span reserves the scaled width and height in layout; the
      // inner span performs only the horizontal visual transform.
      currentRunEl.classList.add("big");
      currentRunEl.style.fontSize = `${height * (font === "b" ? 0.8 : 1)}em`;
      currentRunEl.style.setProperty("--sx", String(width / height));
      currentRunTextEl = document.createElement("span");
      currentRunTextEl.className = "run__ink";
      currentRunEl.appendChild(currentRunTextEl);
    } else {
      currentRunTextEl = currentRunEl;
    }
    currentRunStyleKey = key;
    line.appendChild(currentRunEl);
  }

  currentRunTextEl!.textContent += text;
  if (currentRunEl.classList.contains("big")) {
    // CSS transforms do not affect layout dimensions. Reserve exactly the
    // transformed ink width so width-only and height-only scaling both flow.
    const horizontalScale = width / height;
    currentRunEl.style.width = `${currentRunTextEl!.offsetWidth * horizontalScale}px`;
  }
  growPaper(currentFeedPaper!);
}

function commitLine() {
  currentLineEl = null;
  currentRunEl = null;
  currentRunTextEl = null;
  currentRunStyleKey = "";
  if (currentFeedPaper) growPaper(currentFeedPaper);
}

/** Handles a `new_line` event specifically — as opposed to commitLine(),
 * which other call sites (like appendBlock) use just to flush any
 * in-progress line without necessarily wanting a blank line inserted. */
function handleNewLine() {
  // Guarantee a line div exists even for a blank line (consecutive
  // newlines with no text in between) — otherwise it never gets a DOM
  // element and the blank-line spacing silently disappears.
  ensureLine();
  commitLine();
  playPrintSound();
}

/** Inserts a block-level element (image, QR code, barcode) into the receipt flow. */
function appendBlock(el: Element) {
  playPrintSound();
  ensureFeedPaper();
  commitLine();
  const wrap = document.createElement("div");
  wrap.className = "block";
  wrap.style.textAlign = align;
  wrap.appendChild(el);
  linesContainer!.appendChild(wrap);
  growPaper(currentFeedPaper!);
}

const CSS_PIXELS_PER_MM = 96 / 25.4;
const PAPER_CRUISE_MM_PER_SECOND = 250;
const PAPER_CRUISE_PX_PER_SECOND = PAPER_CRUISE_MM_PER_SECOND * CSS_PIXELS_PER_MM;
const PAPER_ACCEL_SECONDS = 0.12;
const PAPER_DECEL_SECONDS = 0.1;

interface PaperFeedMotion {
  startedAt: number;
  startHeight: number;
  distance: number;
  peakSpeed: number;
  accelSeconds: number;
  cruiseSeconds: number;
  decelSeconds: number;
}

interface PaperFeedState {
  frame: number | null;
  targetHeight: number;
  motion: PaperFeedMotion | null;
  waiters: Array<() => void>;
}

const paperFeedStates = new WeakMap<HTMLElement, PaperFeedState>();

function resolveFeedWaiters(state: PaperFeedState) {
  const waiters = state.waiters.splice(0);
  for (const resolve of waiters) resolve();
}

function positionPrintedContent(paper: HTMLElement, visibleHeight: number, targetHeight: number) {
  const lines = paper.querySelector<HTMLElement>(".feed__lines");
  if (!lines) return;

  // The receipt's bottom edge leaves the slot first. As more paper feeds,
  // reveal content upward instead of painting a finished sheet top-down.
  lines.style.transform = `translateY(${Math.min(0, visibleHeight - targetHeight)}px)`;
}

function measurePaperContent(paper: HTMLElement): number {
  return paper.querySelector<HTMLElement>(".feed__lines")?.scrollHeight ?? paper.scrollHeight;
}

function createPaperFeedMotion(startHeight: number, targetHeight: number): PaperFeedMotion {
  const distance = Math.max(0, targetHeight - startHeight);
  const rampDistanceAtCruise = PAPER_CRUISE_PX_PER_SECOND
    * (PAPER_ACCEL_SECONDS + PAPER_DECEL_SECONDS) / 2;
  const peakSpeed = distance < rampDistanceAtCruise
    ? distance * 2 / (PAPER_ACCEL_SECONDS + PAPER_DECEL_SECONDS)
    : PAPER_CRUISE_PX_PER_SECOND;
  const rampDistance = peakSpeed * (PAPER_ACCEL_SECONDS + PAPER_DECEL_SECONDS) / 2;

  return {
    startedAt: performance.now(),
    startHeight,
    distance,
    peakSpeed,
    accelSeconds: PAPER_ACCEL_SECONDS,
    cruiseSeconds: peakSpeed > 0 ? Math.max(0, distance - rampDistance) / peakSpeed : 0,
    decelSeconds: PAPER_DECEL_SECONDS,
  };
}

function paperFeedDistanceAt(motion: PaperFeedMotion, elapsed: number): number {
  const accelDistance = motion.peakSpeed * motion.accelSeconds / 2;
  if (elapsed < motion.accelSeconds) {
    const acceleration = motion.peakSpeed / motion.accelSeconds;
    return acceleration * elapsed * elapsed / 2;
  }

  const afterAccel = elapsed - motion.accelSeconds;
  if (afterAccel < motion.cruiseSeconds) {
    return accelDistance + motion.peakSpeed * afterAccel;
  }

  const cruiseDistance = motion.peakSpeed * motion.cruiseSeconds;
  const intoDecel = Math.min(motion.decelSeconds, afterAccel - motion.cruiseSeconds);
  const deceleration = motion.peakSpeed / motion.decelSeconds;
  return accelDistance
    + cruiseDistance
    + motion.peakSpeed * intoDecel
    - deceleration * intoDecel * intoDecel / 2;
}

function animatePaperFeed(paper: HTMLElement, state: PaperFeedState, now: number) {
  const motion = state.motion!;
  const elapsedSeconds = (now - motion.startedAt) / 1000;
  const totalSeconds = motion.accelSeconds + motion.cruiseSeconds + motion.decelSeconds;
  const travelled = elapsedSeconds >= totalSeconds
    ? motion.distance
    : paperFeedDistanceAt(motion, elapsedSeconds);
  const nextHeight = Math.min(state.targetHeight, motion.startHeight + travelled);

  paper.style.height = `${nextHeight}px`;
  positionPrintedContent(paper, nextHeight, state.targetHeight);
  playPrintSound();
  pulseLed();

  if (nextHeight < state.targetHeight) {
    state.frame = requestAnimationFrame((timestamp) => animatePaperFeed(paper, state, timestamp));
    return;
  }

  state.frame = null;
  state.motion = null;
  resolveFeedWaiters(state);
}

function growPaper(paper: HTMLElement) {
  let state = paperFeedStates.get(paper);
  if (!state) {
    state = { frame: null, targetHeight: 0, motion: null, waiters: [] };
    paperFeedStates.set(paper, state);
  }

  state.targetHeight = Math.max(state.targetHeight, measurePaperContent(paper));

  // Build the complete receipt at zero height. Its target must remain fixed
  // throughout the feed or newly appended lines make visible content jump.
  if (composingReceipt) {
    positionPrintedContent(paper, 0, state.targetHeight);
    return;
  }

  paper.classList.add("printing");

  if (reduceMotion.matches) {
    if (state.frame !== null) cancelAnimationFrame(state.frame);
    state.frame = null;
    paper.style.height = `${state.targetHeight}px`;
    positionPrintedContent(paper, state.targetHeight, state.targetHeight);
    state.motion = null;
    resolveFeedWaiters(state);
  } else if (state.frame === null) {
    const startHeight = Number.parseFloat(paper.style.height) || 0;
    state.motion = createPaperFeedMotion(startHeight, state.targetHeight);
    state.frame = requestAnimationFrame((timestamp) => animatePaperFeed(paper, state!, timestamp));
  } else {
    const visibleHeight = Number.parseFloat(paper.style.height) || 0;
    positionPrintedContent(paper, visibleHeight, state.targetHeight);
  }
}

function waitForPaperFeed(paper: HTMLElement): Promise<void> {
  growPaper(paper);
  const state = paperFeedStates.get(paper)!;
  if (state.frame === null) return Promise.resolve();
  return new Promise((resolve) => state.waiters.push(resolve));
}

// ---- images ----

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

function renderImage(widthPx: number, heightPx: number, bitsB64: string) {
  if (!widthPx || !heightPx) return;

  const canvas = document.createElement("canvas");
  canvas.width = widthPx;
  canvas.height = heightPx;
  canvas.style.display = "inline-block";
  canvas.className = "receipt-image";

  const ctx = canvas.getContext("2d")!;
  const imgData = ctx.createImageData(widthPx, heightPx);
  const bytes = base64ToBytes(bitsB64);
  const widthBytes = widthPx / 8;

  // ImageData starts fully transparent (all zeros); only paint the "ink"
  // pixels opaque so the paper's own background shows through everywhere
  // else, instead of drawing a solid white rectangle over it.
  for (let y = 0; y < heightPx; y++) {
    for (let x = 0; x < widthPx; x++) {
      const byte = bytes[y * widthBytes + (x >> 3)] ?? 0;
      const bit = (byte >> (7 - (x & 7))) & 1;
      if (!bit) continue;
      const idx = (y * widthPx + x) * 4;
      imgData.data[idx] = 0x2e;
      imgData.data[idx + 1] = 0x2a;
      imgData.data[idx + 2] = 0x22;
      imgData.data[idx + 3] = 255;
    }
  }

  ctx.putImageData(imgData, 0, 0);
  appendBlock(canvas);
}

// ---- QR codes ----

async function renderQrCode(data: string) {
  const canvas = document.createElement("canvas");
  canvas.style.display = "inline-block";
  appendBlock(canvas);

  await new Promise<void>((resolve) => {
    QRCode.toCanvas(
      canvas,
      data,
      { margin: 1, width: 120, color: { dark: "#2e2a22ff", light: "#00000000" } },
      (err: Error | null | undefined) => {
        if (err) console.error("QR render failed", err);
        const paper = canvas.closest(".feed__paper") as HTMLElement | null;
        if (paper) growPaper(paper);
        resolve();
      },
    );
  });
}

// ---- barcodes ----

function mapSymbology(symbology: string): string | null {
  switch (symbology) {
    case "CODE128":
      return "CODE128";
    case "EAN-13":
      return "EAN13";
    case "EAN-8":
      return "EAN8";
    case "UPC-A":
      return "UPC";
    case "CODE39":
      return "CODE39";
    case "ITF":
      return "ITF";
    case "CODABAR":
      return "codabar";
    default:
      return null; // CODE93, GS1 DataBar variants: not supported by the renderer, fall back to a label
  }
}

function renderBarcode(symbology: string, data: string) {
  const format = mapSymbology(symbology);

  if (format) {
    try {
      const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      svg.style.display = "inline-block";
      JsBarcode(svg, data, {
        format,
        displayValue: true,
        width: 2,
        height: 50,
        margin: 8,
        background: "transparent",
        lineColor: "#2e2a22",
      });
      appendBlock(svg);
      return;
    } catch (err) {
      console.error("barcode render failed", err);
    }
  }

  const fallback = document.createElement("div");
  fallback.className = "barcode-fallback";
  fallback.textContent = `[${symbology}] ${data}`;
  appendBlock(fallback);
}

// ---- cash drawer ----

let drawerTimer: ReturnType<typeof setTimeout> | undefined;

function pulseDrawer() {
  drawer.classList.add("kick");
  playCashDrawerSound();
  clearTimeout(drawerTimer);
  drawerTimer = setTimeout(() => drawer.classList.remove("kick"), 220);
}

// ---- cut ----

function makeDraggable(paper: HTMLElement) {
  let pointerId: number | null = null;
  let grabX = 0;
  let grabY = 0;

  paper.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    const rect = paper.getBoundingClientRect();
    pointerId = event.pointerId;
    grabX = event.clientX - rect.left;
    grabY = event.clientY - rect.top;
    paper.style.left = `${rect.left}px`;
    paper.style.top = `${rect.top}px`;
    paper.classList.add("dragging");
    paper.setPointerCapture(event.pointerId);
    hint.classList.add("hidden");
  });

  paper.addEventListener("pointermove", (event) => {
    if (event.pointerId !== pointerId) return;
    const minVisible = 48;
    const nextLeft = Math.min(window.innerWidth - minVisible, Math.max(minVisible - paper.offsetWidth, event.clientX - grabX));
    const nextTop = Math.min(window.innerHeight - minVisible, Math.max(minVisible - paper.offsetHeight, event.clientY - grabY));
    paper.style.left = `${nextLeft}px`;
    paper.style.top = `${nextTop}px`;
  });

  const stopDragging = (event: PointerEvent) => {
    if (event.pointerId !== pointerId) return;
    pointerId = null;
    paper.classList.remove("dragging");
    if (paper.hasPointerCapture(event.pointerId)) paper.releasePointerCapture(event.pointerId);
  };

  paper.addEventListener("pointerup", stopDragging);
  paper.addEventListener("pointercancel", stopDragging);
}

async function handleCut() {
  const paper = currentFeedPaper;

  // A cut command by itself must not expose a new piece of paper.
  if (!paper) {
    stopPrintSound();
    currentFeedPaper = null;
    linesContainer = null;
    resetStyle();
    return;
  }

  // No forced margin here: real print jobs already feed a blank line or
  // two before cutting, and now that blank new_line events render
  // correctly (see handleNewLine), adding another on top just doubled up
  // the gap.
  commitLine();
  playPrintSound();
  await waitForPaperFeed(paper);
  paper.style.height = `${paper.scrollHeight}px`;

  const rect = paper.getBoundingClientRect();

  paper.style.width = `${rect.width}px`;
  paper.style.height = `${rect.height}px`;
  paper.style.left = `${rect.left}px`;
  paper.style.top = `${rect.top}px`;
  paper.classList.add("torn");
  paper.style.zIndex = String(++topReceiptZ);
  paper.dataset.rotation = `${receiptCount % 2 === 0 ? -1.1 : 0.9}deg`;
  receiptDesk.appendChild(paper);
  makeDraggable(paper);
  playCutterSound();

  requestAnimationFrame(() => {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const offsets = [0, -24, 22, -12, 30];
    const x = Math.max(12, Math.min(vw - rect.width - 12, vw / 2 - rect.width / 2 + offsets[receiptCount % offsets.length]));
    const preferredY = Math.max(292, vh * 0.39);
    const y = Math.min(preferredY, vh - Math.min(rect.height, 150) - 24);
    paper.style.left = `${x}px`;
    paper.style.top = `${Math.max(238, y)}px`;
    paper.style.transform = `rotate(${paper.dataset.rotation})`;
    paper.classList.add("settled");
    receiptCount++;
  });

  currentFeedPaper = null;
  linesContainer = null;
  currentLineEl = null;
  currentRunEl = null;
  currentRunTextEl = null;
  currentRunStyleKey = "";
  resetStyle();
}

async function handleEvent(event: PrintEvent) {
  switch (event.type) {
    case "init":
      resetStyle();
      break;
    case "align":
      align = event.value ?? "left";
      break;
    case "style":
      font = event.font ?? "a";
      bold = !!event.bold;
      italic = !!event.italic;
      underline = !!event.underline;
      invert = !!event.invert;
      width = event.width ?? 1;
      height = event.height ?? 1;
      break;
    case "text":
      appendText(event.text ?? "");
      break;
    case "new_line":
      handleNewLine();
      break;
    case "cut":
      await handleCut();
      break;
    case "image":
      renderImage(event.width ?? 0, event.height ?? 0, event.bits ?? "");
      break;
    case "qr_code":
      await renderQrCode(event.data ?? "");
      break;
    case "barcode":
      renderBarcode(event.symbology ?? "barcode", event.data ?? "");
      break;
    case "pulse":
      pulseDrawer();
      break;
  }
}

// Compose each instruction invisibly as it arrives. By the time the cut event
// arrives the sheet is already complete, so feeding can start immediately
// without changing its contents or target height during the animation.
const eventQueue: PrintEvent[] = [];
let processingEvents = false;
const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

async function processEventQueue() {
  if (processingEvents) return;
  processingEvents = true;

  while (eventQueue.length) {
    const event = eventQueue.shift()!;

    if (event.type === "cut") {
      composingReceipt = false;
      await handleEvent(event);
    } else {
      composingReceipt = true;
      await handleEvent(event);
    }
  }

  composingReceipt = false;
  processingEvents = false;
}

function enqueueEvents(events: PrintEvent[]) {
  for (const event of events) {
    if (event.type === "pulse") {
      pulseLed();
      pulseDrawer();
    } else {
      eventQueue.push(event);
    }
  }

  void processEventQueue();
}

// ---- connection ----

let ledTimer: ReturnType<typeof setTimeout> | undefined;

function pulseLed() {
  led.classList.add("active");
  clearTimeout(ledTimer);
  ledTimer = setTimeout(() => led.classList.remove("active"), 350);
}

function connect() {
  const ws = new WebSocket("ws://localhost:9101");

  ws.onopen = () => {
    statusEl.classList.add("connected");
    statusText.textContent = "printer ready";
  };

  ws.onclose = () => {
    statusEl.classList.remove("connected");
    statusText.textContent = "disconnected — retrying…";
    setTimeout(connect, 1000);
  };

  ws.onerror = () => ws.close();

  ws.onmessage = (ev) => {
    try {
      const payload: PrintEvent | PrintEvent[] = JSON.parse(ev.data);
      enqueueEvents(Array.isArray(payload) ? payload : [payload]);
    } catch (err) {
      console.error("bad event from printer daemon", err);
    }
  };
}

connect();
