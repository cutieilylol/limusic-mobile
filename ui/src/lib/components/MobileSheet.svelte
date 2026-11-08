<script lang="ts">
	import type { Snippet } from 'svelte';
	import { fade, fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { onBack } from '$lib/mobile.svelte';

	// A bottom sheet: the phone's version of the desktop's floating side panels (queue, lyrics,
	// comments). Drag the handle down or tap outside to close; the back button closes it too.
	let {
		title,
		onClose,
		tall = false,
		children,
		actions
	}: {
		title: string;
		onClose: () => void;
		tall?: boolean;
		children: Snippet;
		actions?: Snippet;
	} = $props();

	let dy = $state(0);
	let y0 = 0;
	let dragging = $state(false);

	$effect(() =>
		onBack(() => {
			onClose();
			return true;
		})
	);

	function down(e: TouchEvent) {
		y0 = e.touches[0].clientY;
		dragging = true;
	}
	function move(e: TouchEvent) {
		if (!dragging) return;
		dy = Math.max(0, e.touches[0].clientY - y0);
	}
	function up() {
		if (!dragging) return;
		dragging = false;
		if (dy > 110) onClose();
		dy = 0;
	}
</script>

<div class="fixed inset-0 z-[45]">
	<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
	<div class="absolute inset-0 bg-black/50" transition:fade={{ duration: 180 }} onclick={onClose}></div>
	<div
		class="absolute inset-x-0 bottom-0 flex flex-col rounded-t-3xl border-t border-border/60 bg-popover text-popover-foreground shadow-2xl {tall
			? 'h-[88dvh]'
			: 'max-h-[80dvh]'}"
		style="transform: translateY({dy}px); transition: {dragging ? 'none' : 'transform 200ms ease-out'}; padding-bottom: var(--safe-bottom, 0px)"
		transition:fly={{ y: 400, duration: 260, easing: cubicOut }}
	>
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="flex shrink-0 cursor-grab flex-col items-center gap-2 px-4 pb-2 pt-2.5"
			ontouchstart={down}
			ontouchmove={move}
			ontouchend={up}
			ontouchcancel={up}
		>
			<div class="h-1.5 w-10 rounded-full bg-muted-foreground/30"></div>
			<div class="flex w-full items-center gap-2">
				<h2 class="flex-1 text-base font-semibold">{title}</h2>
				{@render actions?.()}
			</div>
		</div>
		<!-- A tall sheet holds a view that scrolls itself (the queue, lyrics, comments each keep their
		     own `flex-1 overflow-y-auto` scroller, which is what they follow the playing row or line
		     in): a bounded column for it to fill. A short one scrolls as a whole. -->
		<div
			class="min-h-0 flex-1 overscroll-contain px-2 pb-3 {tall
				? 'flex flex-col overflow-hidden'
				: 'overflow-y-auto'}"
		>
			{@render children()}
		</div>
	</div>
</div>
