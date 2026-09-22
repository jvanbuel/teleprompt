// The components the demo's motion scenes are drawn with.
//
// teleprompt renders each shot as a composition exactly as long as the
// sentence spoken over it, so every animation here is written against
// `durationInFrames` rather than a fixed number of frames: the same
// component fills a three-second slot and an eight-second one.

import React from "react";
import {
  AbsoluteFill,
  Easing,
  interpolate,
  spring,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";

const INK = "#eef3f8";
const DIM = "#8a96a3";
const ACCENT = "#7cc4ff";
const WARM = "#ffb86b";
const FONT = "Inter, 'Helvetica Neue', Helvetica, Arial, sans-serif";
const MONO = "'JetBrains Mono', 'DejaVu Sans Mono', monospace";

/** How far through its slot the shot is, 0 to 1. */
const useProgress = () => {
  const frame = useCurrentFrame();
  const { durationInFrames } = useVideoConfig();
  return frame / Math.max(1, durationInFrames - 1);
};

/** Fades in at the start of the slot and out at the end, whatever its length. */
const useEnvelope = () => {
  const frame = useCurrentFrame();
  const { durationInFrames, fps } = useVideoConfig();
  const edge = Math.min(fps / 2, durationInFrames / 4);
  return interpolate(
    frame,
    [0, edge, durationInFrames - edge, durationInFrames],
    [0, 1, 1, 0],
    { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
  );
};

export const Backdrop: React.FC = () => {
  const p = useProgress();
  return (
    <AbsoluteFill
      style={{
        background: `radial-gradient(circle at ${30 + p * 40}% ${40 - p * 10}%, #16324a 0%, #0b0d10 60%)`,
      }}
    />
  );
};

export const Title: React.FC<{ title: string; subtitle?: string }> = ({ title, subtitle }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const opacity = useEnvelope();
  const rise = spring({ frame, fps, config: { damping: 200 } });
  const underline = interpolate(frame, [fps * 0.3, fps * 1.2], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.out(Easing.cubic),
  });
  return (
    <AbsoluteFill style={{ justifyContent: "center", alignItems: "center", opacity, fontFamily: FONT }}>
      <div style={{ transform: `translateY(${(1 - rise) * 40}px)`, textAlign: "center" }}>
        <div style={{ fontSize: 120, fontWeight: 800, color: INK, letterSpacing: -2 }}>{title}</div>
        <div
          style={{
            height: 8,
            margin: "24px auto 0",
            width: `${underline * 60}%`,
            background: `linear-gradient(90deg, ${ACCENT}, ${WARM})`,
            borderRadius: 4,
          }}
        />
        {subtitle && <div style={{ fontSize: 44, color: DIM, marginTop: 32 }}>{subtitle}</div>}
      </div>
    </AbsoluteFill>
  );
};

/** A row of stages that light up one after another across the slot. */
export const Pipeline: React.FC<{ steps: string[]; caption?: string }> = ({ steps, caption }) => {
  const frame = useCurrentFrame();
  const { fps, durationInFrames } = useVideoConfig();
  const opacity = useEnvelope();
  // Every stage is on screen by two thirds of the way through, so the
  // whole pipeline holds still for the end of the sentence.
  const per = (durationInFrames * 0.66) / steps.length;
  return (
    <AbsoluteFill style={{ justifyContent: "center", alignItems: "center", opacity, fontFamily: FONT }}>
      <div style={{ display: "flex", alignItems: "center", gap: 28 }}>
        {steps.map((step, i) => {
          const s = spring({ frame: frame - i * per, fps, config: { damping: 14, mass: 0.6 } });
          const lit = frame >= i * per;
          return (
            <React.Fragment key={step}>
              {i > 0 && (
                <div
                  style={{
                    width: 60,
                    height: 6,
                    borderRadius: 3,
                    background: lit ? ACCENT : "#233040",
                    transform: `scaleX(${lit ? s : 0})`,
                    transformOrigin: "left",
                  }}
                />
              )}
              <div
                style={{
                  padding: "36px 40px",
                  borderRadius: 24,
                  fontSize: 42,
                  fontWeight: 700,
                  color: lit ? "#0b0d10" : DIM,
                  background: lit ? (i === steps.length - 1 ? WARM : ACCENT) : "#162030",
                  transform: `scale(${0.8 + 0.2 * s})`,
                  boxShadow: lit ? `0 0 40px ${ACCENT}55` : "none",
                }}
              >
                {step}
              </div>
            </React.Fragment>
          );
        })}
      </div>
      {caption && (
        <div style={{ position: "absolute", bottom: 160, fontSize: 40, color: DIM }}>{caption}</div>
      )}
    </AbsoluteFill>
  );
};

/**
 * Bars that grow to their share of the longest: a picture of narration
 * deciding how long each shot lasts. A row whose `ms` is `"slot"` is this
 * shot's own length — the one number a composition always knows.
 */
export const Durations: React.FC<{ rows: { label: string; ms: number | "slot" }[] }> = ({ rows }) => {
  const frame = useCurrentFrame();
  const { fps, durationInFrames } = useVideoConfig();
  const opacity = useEnvelope();
  const measured = rows.map((r) => ({
    label: r.label,
    ms: r.ms === "slot" ? (durationInFrames / fps) * 1000 : r.ms,
  }));
  const longest = Math.max(...measured.map((r) => r.ms));
  return (
    <AbsoluteFill style={{ justifyContent: "center", padding: "0 220px", opacity, fontFamily: FONT }}>
      {measured.map((row, i) => {
        const grow = spring({ frame: frame - i * fps * 0.6, fps, config: { damping: 200 } });
        return (
          <div key={row.label} style={{ display: "flex", alignItems: "center", margin: "22px 0" }}>
            <div style={{ width: 360, fontSize: 38, color: INK }}>{row.label}</div>
            <div
              style={{
                height: 54,
                borderRadius: 12,
                width: `${(row.ms / longest) * 100 * grow * 0.7}%`,
                background: `linear-gradient(90deg, ${ACCENT}, ${WARM})`,
              }}
            />
            <div style={{ marginLeft: 24, fontSize: 34, color: DIM, fontFamily: MONO, opacity: grow }}>
              {(row.ms / 1000).toFixed(1)}s
            </div>
          </div>
        );
      })}
    </AbsoluteFill>
  );
};

/** A line of code typed out across the first half of the slot. */
export const Code: React.FC<{ lines: string[] }> = ({ lines }) => {
  const p = useProgress();
  const opacity = useEnvelope();
  const text = lines.join("\n");
  const shown = Math.round(Math.min(1, p * 2) * text.length);
  return (
    <AbsoluteFill style={{ justifyContent: "center", alignItems: "center", opacity }}>
      <pre
        style={{
          fontFamily: MONO,
          fontSize: 40,
          lineHeight: 1.5,
          color: INK,
          background: "#11161c",
          border: "1px solid #233040",
          borderRadius: 20,
          padding: "48px 64px",
          minWidth: 1100,
          whiteSpace: "pre",
        }}
      >
        {text.slice(0, shown)}
        <span style={{ color: ACCENT, opacity: Math.floor(p * 20) % 2 ? 1 : 0.2 }}>▍</span>
      </pre>
    </AbsoluteFill>
  );
};

/** One sentence, with a progress bar that ends exactly when the slot does. */
export const Caption: React.FC<{ text: string }> = ({ text }) => {
  const p = useProgress();
  const opacity = useEnvelope();
  return (
    <AbsoluteFill style={{ justifyContent: "center", alignItems: "center", opacity, fontFamily: FONT }}>
      <div style={{ fontSize: 64, color: INK, maxWidth: 1400, textAlign: "center", lineHeight: 1.3 }}>
        {text}
      </div>
      <div style={{ position: "absolute", bottom: 0, left: 0, height: 10, width: `${p * 100}%`, background: ACCENT }} />
    </AbsoluteFill>
  );
};
