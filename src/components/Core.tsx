import { memo, useRef } from "react";
import type { VoiceLanguage, VoicePhase } from "../domain/voice";
import { voiceCopy, voicePhaseCopyKey } from "../domain/voice-copy";
import { useOrbit } from "../hooks/useOrbit";

const ticks = Array.from({ length: 48 }, (_, index) => ({
	id: `tick-${index}`,
	angle: index * 7.5,
	major: index % 4 === 0,
}));

export const Core = memo(function Core({
	enabled,
	phase,
	language,
	notice,
}: {
	enabled: boolean;
	phase: VoicePhase;
	language: VoiceLanguage;
	notice?: string;
}) {
	const outer = useRef<HTMLDivElement>(null);
	const inner = useRef<HTMLDivElement>(null);
	const active = ["listening", "transcribing", "routing", "thinking", "speaking"].includes(phase);
	useOrbit(outer, enabled, active ? 0.75 : 1, 120_000, true);
	useOrbit(inner, enabled, active ? 1.3 : 1, 56_000);

	return (
		<section
			className="core-panel"
			aria-label={voiceCopy(language, "voice.native_status")}
			data-state={phase}
		>
			<div className="core-stage">
				<div className="core-halo" aria-hidden="true" />
				<div className="orbital-layer" data-orbit="outer" ref={outer} aria-hidden="true">
					<svg aria-hidden="true" viewBox="0 0 400 400">
						<circle className="orb-guide" cx="200" cy="200" r="181" />
						{ticks.map(({ id, angle, major }) => (
							<line
								key={id}
								x1="200"
								y1="26"
								x2="200"
								y2={major ? 34 : 29}
								transform={`rotate(${angle} 200 200)`}
								className={major ? "orb-tick major" : "orb-tick"}
							/>
						))}
					</svg>
				</div>
				<div className="orbital-layer" data-orbit="inner" ref={inner} aria-hidden="true">
					<svg aria-hidden="true" viewBox="0 0 400 400" fill="none">
						<circle className="orb-track" cx="200" cy="200" r="157" />
						<circle
							className="orb-arc"
							cx="200"
							cy="200"
							r="157"
							strokeDasharray="190 120 55 310"
						/>
						<circle className="orb-pin" cx="200" cy="43" r="2.4" />
					</svg>
				</div>
				<div className="core-identity">
					<h1>S.A.T.U.R.D.A.Y</h1>
					<span className="core-state" role="status">
						{voiceCopy(language, voicePhaseCopyKey(phase))}
					</span>
				</div>
			</div>
			<p className="core-caption" aria-live="polite">
				{notice ?? voiceCopy(language, voicePhaseCopyKey(phase))}
			</p>
		</section>
	);
});
