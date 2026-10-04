import { type RefObject, useLayoutEffect, useRef } from "react";

/** Own one compositor timeline; neither state nor pause creates a new player. */
export function useOrbit(
	ref: RefObject<HTMLDivElement | null>,
	enabled: boolean,
	targetRate: number,
	duration: number,
	reverse = false,
) {
	const player = useRef<Animation | null>(null);
	const rate = useRef(1);
	useLayoutEffect(() => {
		if (!ref.current) return;
		const animation = ref.current.animate(
			[
				{ transform: "rotate(0deg)" },
				{ transform: `rotate(${reverse ? -360 : 360}deg)` },
			],
			{ duration, iterations: Infinity, easing: "linear", fill: "both" },
		);
		animation.pause();
		player.current = animation;
		return () => {
			animation.cancel();
			player.current = null;
		};
	}, [ref, duration, reverse]);
	useLayoutEffect(() => {
		const animation = player.current;
		if (!animation) return;
		if (enabled) animation.play();
		else animation.pause();
	}, [enabled]);
	useLayoutEffect(() => {
		const animation = player.current;
		if (!animation) return;
		if (!enabled) {
			rate.current = targetRate;
			animation.updatePlaybackRate(targetRate);
			return;
		}
		let frame = 0;
		let previous: number | null = null;
		const easeRate = (now: number) => {
			const delta =
				previous === null ? 0 : Math.min(40, Math.max(0, now - previous));
			previous = now;
			rate.current +=
				(targetRate - rate.current) * (1 - Math.exp(-delta / 150));
			if (Math.abs(targetRate - rate.current) < 0.001)
				rate.current = targetRate;
			// Unlike duration changes, this API preserves the current playback position.
			animation.updatePlaybackRate(rate.current);
			if (rate.current !== targetRate) frame = requestAnimationFrame(easeRate);
		};
		if (rate.current !== targetRate) frame = requestAnimationFrame(easeRate);
		return () => cancelAnimationFrame(frame);
	}, [enabled, targetRate]);
}
