<script lang="ts">
	import { fade } from 'svelte/transition';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { PlayIcon, PauseIcon, NextIcon, MusicNote01Icon, FavouriteIcon } from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import { np, playback, toggleNowPlayingRating } from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import { haptic, swipe } from '$lib/mobile.svelte';
	import Marquee from './Marquee.svelte';

	// The phone's player bar: what's playing, play/pause and next, a hairline of progress along the
	// bottom. Tap it (or swipe it up) for the full player; swipe it sideways to skip, the way
	// YouTube Music and Spotify's mini players work.
	let dx = $state(0);
	const pct = $derived(playback.duration ? (playback.position / playback.duration) * 100 : 0);
	const isControl = (t: EventTarget | null) => !!(t as HTMLElement | null)?.closest?.('button');

	function onSwipe(dir: 'left' | 'right' | 'up' | 'down') {
		if (dir === 'left') {
			haptic();
			api.nextTrack();
		} else if (dir === 'right') {
			haptic();
			api.prevTrack();
		} else if (dir === 'up') {
			np.open = true;
		}
	}
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div
	class="relative mx-2 mb-1.5 overflow-hidden rounded-xl border border-border/60 bg-card shadow-lg"
	onclick={(e) => {
		if (!isControl(e.target)) np.open = true;
	}}
	{@attach swipe({
		onSwipe,
		onMove: (x) => (dx = x),
		onEnd: () => (dx = 0)
	})}
>
	<div
		class="flex items-center gap-3 py-2 pl-2 pr-1"
		style="transform: translateX({dx * 0.4}px); opacity: {1 - Math.min(Math.abs(dx) / 400, 0.4)}; transition: {dx
			? 'none'
			: 'transform 180ms ease-out, opacity 180ms'}"
	>
		{#key playback.now?.videoId}
			{#if playback.now?.thumbnail}
				<img
					src={thumb(playback.now.thumbnail, 120)}
					alt=""
					style="max-width:none"
					class="h-11 w-11 shrink-0 rounded-lg object-cover"
					in:fade={{ duration: 200 }}
				/>
			{:else}
				<div
					class="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground/50"
				>
					<HugeiconsIcon icon={MusicNote01Icon} class="h-5 w-5" />
				</div>
			{/if}
		{/key}
		<div class="min-w-0 flex-1">
			<Marquee text={playback.now?.title ?? t('player.not_playing')} class="text-sm font-medium" />
			<!-- Plain text, not artist links: a tap anywhere on the bar opens the player. -->
			<p class="truncate text-xs text-muted-foreground">{playback.now?.artists ?? ''}</p>
		</div>
		{#if playback.now && !api.isLocalId(playback.now.videoId)}
			<button
				class="inline-flex size-10 items-center justify-center rounded-full active:bg-muted"
				onclick={() => toggleNowPlayingRating()}
				aria-label={t('common.like')}
			>
				<HugeiconsIcon
					icon={FavouriteIcon}
					class="h-5 w-5 {playback.rating === 'like' ? 'fill-current text-primary' : 'text-muted-foreground'}"
				/>
			</button>
		{/if}
		<button
			class="inline-flex size-11 items-center justify-center rounded-full active:bg-muted"
			onclick={() => api.togglePause()}
			aria-label={playback.paused ? t('player.play') : t('player.pause')}
		>
			<HugeiconsIcon icon={PauseIcon} altIcon={PlayIcon} showAlt={playback.paused} class="h-6 w-6" />
		</button>
		<button
			class="inline-flex size-10 items-center justify-center rounded-full active:bg-muted"
			onclick={() => api.nextTrack()}
			aria-label={t('player.next')}
		>
			<HugeiconsIcon icon={NextIcon} class="h-5 w-5" />
		</button>
	</div>
	<div class="absolute inset-x-2 bottom-0 h-0.5 overflow-hidden rounded-full bg-muted">
		<div class="h-full bg-primary" style="width:{pct}%"></div>
	</div>
</div>
