import { memo, type RefObject, useEffect, useRef } from "react";
import { type AudioSignal, SILENT_AUDIO } from "../domain/audio";
import { useOrbit } from "../hooks/useOrbit";
export type CoreState = "standby" | "listening" | "processing" | "responding";
type Point = readonly [number, number, number];
// Static geometry is computed once, not rebuilt on every animation frame.
const paths: Point[][] = [];
for (let latitude = -4; latitude <= 4; latitude++) {
	const phi = (latitude * Math.PI) / 11;
	paths.push(
		Array.from({ length: 49 }, (_, step) => {
			const theta = (step * Math.PI) / 24;
			return [
				Math.cos(theta) * Math.cos(phi) * 90,
				Math.sin(phi) * 90,
				Math.sin(theta) * Math.cos(phi) * 90,
			] as const;
		}),
	);
}
for (let longitude = 0; longitude < 12; longitude++) {
	const theta = (longitude * Math.PI) / 6;
	paths.push(
		Array.from({ length: 33 }, (_, step) => {
			const phi = (step * Math.PI) / 32 - Math.PI / 2;
			return [
				Math.cos(theta) * Math.cos(phi) * 90,
				Math.sin(phi) * 90,
				Math.sin(theta) * Math.cos(phi) * 90,
			] as const;
		}),
	);
}
const points: Point[] = Array.from({ length: 120 }, (_, index) => {
	const y = 1 - (index / 119) * 2,
		radius = Math.sqrt(1 - y * y),
		theta = index * 2.399963;
	return [Math.cos(theta) * radius * 90, y * 90, Math.sin(theta) * radius * 90];
});
const ticks = Array.from({ length: 48 }, (_, index) => ({
	id: `tick-${index}`,
	angle: index * 7.5,
	major: index % 4 === 0,
}));
const ParticleField = memo(function ParticleField({
	enabled,
	accent,
	audio,
}: {
	enabled: boolean;
	accent: "cyan" | "amber";
	audio: RefObject<AudioSignal>;
}) {
	const ref = useRef<HTMLCanvasElement>(null);
	const rotation = useRef(0);
	const energy = useRef(0);
	const cooldown = useRef(0);
	const pulses = useRef(
		Array.from({ length: 3 }, () => ({ phase: 1, strength: 0 })),
	);
	useEffect(() => {
		const canvas = ref.current,
			ctx = canvas?.getContext("2d");
		if (!canvas || !ctx) return;
		const resize = () => {
			const size = Math.max(
				1,
				Math.round(
					canvas.getBoundingClientRect().width *
						Math.min(devicePixelRatio || 1, 2),
				),
			);
			if (canvas.width !== size || canvas.height !== size) {
				canvas.width = size;
				canvas.height = size;
				ctx.setTransform(
					size / 400,
					0,
					0,
					size / 400,
					(size * 72) / 400,
					(size * 72) / 400,
				);
			}
		};
		resize();
		const observer = new ResizeObserver(resize);
		observer.observe(canvas);
		return () => observer.disconnect();
	}, []);
	useEffect(() => {
		const canvas = ref.current,
			ctx = canvas?.getContext("2d");
		if (!canvas || !ctx) return;
		const rgb = accent === "cyan" ? "120,232,245" : "239,197,142";
		const projected = new Float32Array(points.length * 3);
		let frame = 0,
			last: number | null = null;
		const draw = (now: number) => {
			const dt = last === null ? 16 : Math.min(40, Math.max(0, now - last));
			const sound = audio.current ?? SILENT_AUDIO;
			if (enabled) {
				const target = Math.max(0, Math.min(1, sound.level)) ** 0.65;
				cooldown.current = Math.max(0, cooldown.current - dt);
				for (const pulse of pulses.current)
					pulse.phase = Math.min(1, pulse.phase + dt / 650);
				if (
					sound.active &&
					target - energy.current > 0.12 &&
					cooldown.current === 0
				) {
					const pulse = pulses.current.find((pulse) => pulse.phase === 1);
					if (pulse) {
						pulse.phase = 0;
						pulse.strength = target;
						cooldown.current = 420;
					}
				}
				energy.current +=
					(target - energy.current) *
					(1 - Math.exp(-dt / (target > energy.current ? 24 : 140)));
				if (last !== null)
					rotation.current += dt * 0.00009 * (1 + energy.current * 2.5);
			}
			last = now;
			const cos = Math.cos(rotation.current),
				sin = Math.sin(rotation.current);
			const project = ([x, y, z]: Point): Point => {
				const xx = x * cos - z * sin,
					zz = x * sin + z * cos,
					yy = y * 0.94 - zz * 0.34,
					depth = y * 0.34 + zz * 0.94;
				const deformation =
					Math.sin(y * 0.035 + rotation.current * 3) * (0.4 + sound.low * 0.6) +
					Math.sin(z * 0.06 - rotation.current * 4) * sound.mid * 0.45;
				const scale =
					(340 / (340 + depth)) *
					(1 + energy.current * (0.28 + deformation * 0.1));
				return [128 + xx * scale, 128 + yy * scale, depth];
			};
			ctx.clearRect(-72, -72, 400, 400);
			ctx.beginPath();
			for (const path of paths)
				path.forEach((p, index) => {
					const [x, y] = project(p);
					if (index === 0) ctx.moveTo(x, y);
					else ctx.lineTo(x, y);
				});
			ctx.lineWidth = 0.55;
			ctx.strokeStyle = `rgba(${rgb},${0.17 + energy.current * 0.55})`;
			ctx.stroke();
			points.forEach((p, index) => {
				const q = project(p);
				projected.set(q, index * 3);
			});
			for (const front of [false, true]) {
				ctx.beginPath();
				for (let i = 0; i < points.length; i++) {
					const x = projected[i * 3],
						y = projected[i * 3 + 1],
						z = projected[i * 3 + 2];
					if (z < 0 !== front) continue;
					const radius =
						(front ? 1.1 : 0.65) + energy.current * (front ? 1.2 : 0.5);
					ctx.moveTo(x + radius, y);
					ctx.arc(x, y, radius, 0, Math.PI * 2);
				}
				ctx.fillStyle = `rgba(${rgb},${front ? 0.72 : 0.22})`;
				ctx.fill();
			}
			if (sound.active || energy.current > 0.001) {
				for (let layer = 0; layer < 3; layer++) {
					ctx.beginPath();
					for (let i = 0; i <= 128; i++) {
						const a = (i / 128) * Math.PI * 2;
						const wave =
							Math.sin(a * 3 + rotation.current * 4 + layer * 0.7) *
								(0.55 + sound.low * 0.3) +
							Math.sin(a * 7 - rotation.current * 3 + layer) *
								(0.15 + sound.mid * 0.35) +
							Math.sin(a * 13 + layer) * sound.high * 0.2;
						const radius =
							122 +
							layer * 10 +
							energy.current * (12 + wave * (18 + layer * 3));
						const x = 128 + Math.cos(a) * radius,
							y = 128 + Math.sin(a) * radius;
						if (i === 0) ctx.moveTo(x, y);
						else ctx.lineTo(x, y);
					}
					ctx.closePath();
					ctx.lineWidth = 7;
					ctx.strokeStyle = `rgba(${rgb},${energy.current * 0.07})`;
					ctx.stroke();
					ctx.lineWidth = 1 + energy.current * (2 - layer * 0.4);
					ctx.strokeStyle = `rgba(${rgb},${(sound.active ? 0.08 : 0) + energy.current * (0.85 - layer * 0.18)})`;
					ctx.stroke();
				}
				// Speech-driven radial detail, not a canned looping equalizer.
				ctx.beginPath();
				for (let i = 0; i < 64; i++) {
					const a = (i / 64) * Math.PI * 2;
					const band =
						i % 3 === 0 ? sound.low : i % 3 === 1 ? sound.mid : sound.high;
					const inner = 157 + energy.current * 4;
					const length =
						energy.current *
						(5 + band * 22 + (1 + Math.sin(a * 5 + rotation.current * 3)) * 5);
					ctx.moveTo(128 + Math.cos(a) * inner, 128 + Math.sin(a) * inner);
					ctx.lineTo(
						128 + Math.cos(a) * (inner + length),
						128 + Math.sin(a) * (inner + length),
					);
				}
				ctx.lineWidth = 1.4;
				ctx.strokeStyle = `rgba(${rgb},${energy.current * 0.75})`;
				ctx.stroke();
			}
			for (const pulse of pulses.current) {
				if (pulse.phase >= 1) continue;
				ctx.beginPath();
				ctx.arc(128, 128, 135 + pulse.phase * 52, 0, Math.PI * 2);
				ctx.lineWidth = 2;
				ctx.strokeStyle = `rgba(${rgb},${pulse.strength * (1 - pulse.phase) ** 2 * 0.3})`;
				ctx.stroke();
			}
			if (enabled) frame = requestAnimationFrame(draw);
		};
		draw(performance.now());
		// A paused canvas must still redraw after resize; no permanent frame loop is needed.
		const observer = new ResizeObserver(() => {
			if (!enabled) draw(performance.now());
		});
		observer.observe(canvas);
		return () => {
			cancelAnimationFrame(frame);
			observer.disconnect();
		};
	}, [enabled, accent, audio]);
	return (
		<canvas
			className="core-particles"
			ref={ref}
			tabIndex={-1}
			aria-hidden="true"
		/>
	);
});
export const Core = memo(function Core({
	enabled,
	accent,
	state,
	audio,
	notice,
}: {
	enabled: boolean;
	accent: "cyan" | "amber";
	state: CoreState;
	audio: RefObject<AudioSignal>;
	notice?: string;
}) {
	const outer = useRef<HTMLDivElement>(null),
		inner = useRef<HTMLDivElement>(null);
	useOrbit(outer, enabled, 1, 120000, true);
	useOrbit(
		inner,
		enabled,
		state === "processing"
			? 1.8
			: state === "listening"
				? 0.7
				: 1,
		56000,
	);
	const labels: Record<CoreState, string> = {
		standby: "At your command.",
		listening: "Listening locally · no recording",
		processing: "Preparing a scripted reply…",
		responding: "Your preview is ready.",
	};
	return (
		<section
			className="core-panel"
			aria-label="Assistant core"
			data-state={state}
		>
			<div className="core-stage">
				<div className="core-halo" aria-hidden="true" />
				<div
					className="orbital-layer"
					data-orbit="outer"
					ref={outer}
					aria-hidden="true"
				>
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
				<div
					className="orbital-layer"
					data-orbit="inner"
					ref={inner}
					aria-hidden="true"
				>
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
				<ParticleField enabled={enabled} accent={accent} audio={audio} />
				<div className="core-identity">
					<h1>S.A.T.U.R.D.A.Y</h1>
					<span className="core-state">
						{state === "standby" ? "READY" : state.toUpperCase()}
					</span>
				</div>
			</div>
			<p className="core-caption" aria-live="polite">
				{notice || labels[state]}
			</p>
		</section>
	);
});
