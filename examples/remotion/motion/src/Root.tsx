// An ordinary Remotion project, which teleprompt renders by composition id.
// The durations registered here are only defaults: teleprompt renders each
// shot as long as the sentence spoken over it.

import React from "react";
import { AbsoluteFill, Composition } from "remotion";
import { Backdrop, Caption, Code, Durations, Pipeline, Title } from "./components";

const withBackdrop =
  <P extends object>(Scene: React.FC<P>): React.FC<P> =>
  (props) => (
    <AbsoluteFill style={{ background: "#0b0d10" }}>
      <Backdrop />
      <Scene {...props} />
    </AbsoluteFill>
  );

const frame = { fps: 30, width: 1920, height: 1080, durationInFrames: 150 };

export const Root: React.FC = () => (
  <>
    <Composition id="Title" component={withBackdrop(Title)} {...frame} defaultProps={{ title: "teleprompt" }} />
    <Composition id="Pipeline" component={withBackdrop(Pipeline)} {...frame} defaultProps={{ steps: ["a", "b"] }} />
    <Composition id="Code" component={withBackdrop(Code)} {...frame} defaultProps={{ lines: ["hello"] }} />
    <Composition id="Caption" component={withBackdrop(Caption)} {...frame} defaultProps={{ text: "Hello" }} />
    <Composition id="Durations" component={withBackdrop(Durations)} {...frame} defaultProps={{ rows: [] }} />
  </>
);
