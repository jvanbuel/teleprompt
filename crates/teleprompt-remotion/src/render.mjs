// Renders a session's shots from the project's own entry point, one clip
// each. Written into the project by teleprompt so that `@remotion/*`
// resolves from the project's node_modules; the job is argv[2].
import { readFileSync } from "node:fs";
import { bundle } from "@remotion/bundler";
import { renderMedia, selectComposition } from "@remotion/renderer";

const job = JSON.parse(readFileSync(process.argv[2], "utf8"));
const browserExecutable = job.browser;
const serveUrl = await bundle({ entryPoint: job.entry });

for (const shot of job.shots) {
  const inputProps = shot.props;
  const found = await selectComposition({ serveUrl, id: shot.composition, inputProps, browserExecutable });
  // teleprompt owns the length and the frame; the composition owns the rest.
  const composition = { ...found, durationInFrames: shot.frames, fps: job.fps, width: job.width, height: job.height };
  await renderMedia({ composition, serveUrl, codec: "h264", inputProps, outputLocation: shot.out, browserExecutable, logLevel: "error" });
}
