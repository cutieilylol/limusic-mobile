<script lang="ts">
	import { fly, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { beforeNavigate } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ArrowDown01Icon,
		FavouriteIcon,
		Add01Icon,
		PreviousIcon,
		NextIcon,
		PlayIcon,
		PauseIcon,
		ShuffleIcon,
		RepeatIcon,
		RepeatOne01Icon,
		Queue01Icon,
		Mic01Icon,
		Comment01Icon,
		Moon02Icon,
		Share08Icon,
		SlidersHorizontalIcon,
		MusicNote01Icon,
		InfinityIcon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import {
		cycleRepeat,
		openAddToPlaylist,
		playback,
		np,
		toggleNowPlayingRating,
		toast,
		ui
	} from '$lib/player.svelte';
	import { canSwitch, claimVideo, parkVideo, pickVersion, showVideo, videoSide } from '$lib/video.svelte';
	import { isMissingThumb, thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import {
		haptic,
		keepScreenOn,
		mobile,
		onBack,
		isAndroid,
		openEqualizer,
		share,
		swipe
	} from '$lib/mobile.svelte';
	import ArtistLine from './ArtistLine.svelte';
	import Marquee from './Marquee.svelte';
	import TrackMenu from './TrackMenu.svelte';
	import QueueList from './QueueList.svelte';
	import LyricsView from './LyricsView.svelte';
	import CommentsView from './CommentsView.svelte';
	import MobileSheet from './MobileSheet.svelte';
	import SleepTimer from './SleepTimer.svelte';

	// The phone's full-screen player: what the desktop splits between the now-playing view and the
	// player bar, stacked for a thumb. Queue, lyrics and comments open as sheets over it.
	beforeNavigate(() => {
		mobile.sheet = null;
		np.open = false;
	});
	$effect(() =>
		onBack(() => {
			np.open = false;
			return true;
		})
	);

	// Lyrics read along: the screen stays on while they are up, like YouTube Music's lyrics view.
	$effect(() => {
		const on = mobile.sheet === 'lyrics';
		keepScreenOn(on).catch(() => {});
		return () => {
			if (on) keepScreenOn(false).catch(() => {});
		};
	});

	const fmt = (secs: number) => {
		if (!secs || secs < 0) return '0:00';
		const s = Math.floor(secs);
		const h = Math.floor(s / 3600);
		const m = Math.floor((s % 3600) / 60);
		const mm = h ? m.toString().padStart(2, '0') : `${m}`;
		return `${h ? `${h}:` : ''}${mm}:${(s % 60).toString().padStart(2, '0')}`;
	};

	const shuffleOn = $derived(playback.queue.shuffle ?? false);
	const repeat = $derived(playback.queue.repeat ?? 'off');
	const currentSong = $derived.by(() => {
		const cur = playback.queue.items[playback.queue.currentIndex];
		return cur?.video_id === playback.now?.videoId ? cur : null;
	});
	const autoplayTrack = $derived(!!currentSong?.autoplay);
	const local = $derived(!!playback.now && api.isLocalId(playback.now.videoId));

	let seekDrag = $state<number | null>(null);
	const shown = $derived(seekDrag ?? playback.position);

	// Cover: crisp first, then smaller sizes, then a note (see NowPlaying's `srcs`).
	let attempt = $state(0);
	$effect(() => {
		playback.now?.thumbnail;
		attempt = 0;
	});
	const srcs = $derived([720, 400, 120].map((px) => thumb(playback.now?.thumbnail, px)));

	let justLiked = $state(false);
	function toggleLike() {
		if (playback.rating !== 'like') {
			justLiked = true;
			haptic();
		}
		toggleNowPlayingRating();
	}

	// The artwork follows the finger sideways and a release past the threshold skips, the swipe on
	// YouTube Music's player. Down anywhere on the top half closes the player.
	let artDx = $state(0);
	let viewDy = $state(0);
	function onArtSwipe(dir: 'left' | 'right' | 'up' | 'down') {
		if (dir === 'left') {
			haptic();
			api.nextTrack();
		} else if (dir === 'right') {
			haptic();
			api.prevTrack();
		} else if (dir === 'down') {
			np.open = false;
		}
	}

	function addToPlaylist() {
		const now = playback.now!;
		openAddToPlaylist(
			currentSong ?? {
				video_id: now.videoId,
				title: now.title,
				artists: now.artists,
				artist_id: now.artistId,
				thumbnail: now.thumbnail,
				duration: now.duration
			}
		);
	}

	function shareNow() {
		const now = playback.now;
		if (!now || local) return;
		share(`https://music.youtube.com/watch?v=${now.videoId}`, `${now.title} · ${now.artists}`);
	}

	async function eq() {
		try {
			await openEqualizer();
		} catch {
			toast.error(t('mobile.no_equalizer'));
		}
	}

	const sleepOn = $derived(mobile.sleep.at !== null || mobile.sleep.endOfTrack);
	const round =
		'inline-flex size-12 items-center justify-center rounded-full transition-colors active:bg-foreground/10';
	const pill =
		'flex min-w-0 flex-1 flex-col items-center gap-1 rounded-2xl px-1 py-2 text-[11px] text-muted-foreground active:bg-foreground/10';
</script>

<div
	class="fixed inset-0 z-40 flex flex-col overflow-hidden bg-background text-foreground"
	style="transform: translateY({viewDy}px); transition: {viewDy ? 'none' : 'transform 200ms ease-out'}"
	transition:fly={{ y: 800, duration: 280, easing: cubicOut, opacity: 1 }}
>
	<!-- The cover lights the room: a blurred copy behind everything, darkened for contrast. -->
	{#if playback.now?.thumbnail}
		{#key playback.now.videoId}
			<div
				class="pointer-events-none absolute inset-0 -z-10 scale-125 bg-cover bg-center opacity-60 blur-3xl saturate-150"
				style="background-image:url({thumb(playback.now.thumbnail, 120)})"
			></div>
		{/key}
		<div class="pointer-events-none absolute inset-0 -z-10 bg-gradient-to-b from-background/40 via-background/70 to-background"></div>
	{/if}

	<!-- Header: drag it down to close too. -->
	<div
		class="flex shrink-0 items-center gap-1 px-2"
		style="padding-top: var(--safe-top, 0px); min-height: calc(3.5rem + var(--safe-top, 0px))"
		{@attach swipe({
			axis: 'y',
			onSwipe: (d) => d === 'down' && (np.open = false),
			onMove: (_x, y) => (viewDy = Math.max(0, y)),
			onEnd: () => (viewDy = 0)
		})}
	>
		<button class="{round} size-11" onclick={() => (np.open = false)} aria-label={t('player.minimize_player')}>
			<HugeiconsIcon icon={ArrowDown01Icon} class="h-6 w-6" />
		</button>
		<div class="flex flex-1 justify-center">
			{#if canSwitch()}
				<div class="flex rounded-full bg-foreground/10 p-0.5">
					{#each [false, true] as on (on)}
						<button
							type="button"
							onclick={() => pickVersion(on)}
							aria-pressed={videoSide() === on}
							class="rounded-full px-4 py-1 text-sm font-medium transition-colors {videoSide() === on
								? 'bg-background text-foreground shadow-sm'
								: 'text-muted-foreground'}"
						>
							{on ? t('player.video') : t('player.song')}
						</button>
					{/each}
				</div>
			{:else}
				<span class="text-xs font-medium uppercase tracking-widest text-muted-foreground">
					{t('mobile.now_playing')}
				</span>
			{/if}
		</div>
		{#if currentSong}
			<TrackMenu
				song={currentSong}
				linksOnly
				playlistId={playback.queue.sourceId}
				queueIndex={playback.queue.currentIndex}
				onAdd={() => openAddToPlaylist(currentSong!)}
				triggerClass="inline-flex size-11 items-center justify-center rounded-full text-foreground active:bg-foreground/10"
			/>
		{:else}
			<div class="size-11"></div>
		{/if}
	</div>

	<!-- Artwork (or the music video). -->
	<div
		class="flex min-h-0 flex-1 items-center justify-center px-6 py-2"
		{@attach swipe({
			onSwipe: onArtSwipe,
			onMove: (x, y) => {
				artDx = x;
				viewDy = Math.max(0, y);
			},
			onEnd: () => {
				artDx = 0;
				viewDy = 0;
			}
		})}
	>
		<div
			class="relative aspect-square w-full max-w-[min(100%,calc(100dvh-25rem))]"
			style="transform: translateX({artDx * 0.6}px) rotate({artDx * 0.01}deg); transition: {artDx
				? 'none'
				: 'transform 200ms ease-out'}"
		>
			{#if showVideo()}
				<div
					class="absolute inset-0 flex items-center justify-center overflow-hidden rounded-2xl bg-black"
					{@attach (box: HTMLElement) => {
						if (ui.theaterOpen) return;
						claimVideo(box);
						return () => parkVideo(box);
					}}
				></div>
			{:else}
				{#key playback.now?.videoId}
					<div in:scale={{ start: 0.94, duration: 320, easing: cubicOut }} class="absolute inset-0">
						{#if srcs[attempt] && attempt < srcs.length}
							<img
								src={srcs[attempt]}
								alt=""
								onerror={() => attempt++}
								onload={(e) => isMissingThumb(e) && attempt++}
								style={srcs[2] ? `background-image:url(${srcs[2]})` : undefined}
								class="h-full w-full rounded-2xl bg-cover object-cover shadow-2xl"
							/>
						{:else}
							<div class="flex h-full w-full items-center justify-center rounded-2xl bg-muted text-muted-foreground/40">
								<HugeiconsIcon icon={MusicNote01Icon} class="h-16 w-16" />
							</div>
						{/if}
					</div>
				{/key}
			{/if}
		</div>
	</div>

	<!-- Title, artist, like, add. -->
	<div class="flex shrink-0 items-center gap-2 px-6 pt-2">
		<div class="min-w-0 flex-1">
			<div class="flex items-center gap-1.5">
				<Marquee text={playback.now?.title ?? ''} class="text-xl font-bold" />
				{#if autoplayTrack}
					<HugeiconsIcon icon={InfinityIcon} class="h-4 w-4 shrink-0 text-muted-foreground" />
				{/if}
			</div>
			<ArtistLine
				runs={playback.now?.artistRuns}
				text={playback.now?.artists ?? ''}
				marquee
				class="block max-w-full text-sm text-muted-foreground"
			/>
		</div>
		{#if !local}
			<button class="{round} size-11" onclick={toggleLike} aria-label={t('common.like')}>
				<span class="inline-flex" class:animate-heart-pop={justLiked} onanimationend={() => (justLiked = false)}>
					<HugeiconsIcon
						icon={FavouriteIcon}
						class="h-6 w-6 {playback.rating === 'like' ? 'fill-current text-primary' : ''}"
					/>
				</span>
			</button>
		{/if}
		<button class="{round} size-11" onclick={addToPlaylist} aria-label={t('player.save_to_playlist')}>
			<HugeiconsIcon icon={Add01Icon} class="h-6 w-6" />
		</button>
	</div>

	<!-- Seek. -->
	<div class="shrink-0 px-6 pt-3">
		<input
			type="range"
			class="range range-touch w-full"
			style="--pct:{playback.duration ? (shown / playback.duration) * 100 : 0}%"
			min="0"
			max={playback.duration || 0}
			step="0.5"
			value={shown}
			oninput={(e) => (seekDrag = Number((e.target as HTMLInputElement).value))}
			onchange={(e) => {
				const v = Number((e.target as HTMLInputElement).value);
				playback.position = v;
				seekDrag = null;
				api.seek(v);
			}}
			aria-label={t('player.seek')}
		/>
		<div class="mt-1 flex justify-between text-xs tabular-nums text-muted-foreground">
			<span>{fmt(shown)}</span>
			<span>{fmt(playback.duration)}</span>
		</div>
	</div>

	<!-- Transport. -->
	<div class="flex shrink-0 items-center justify-between px-5 pt-1">
		<button class={round} onclick={() => api.toggleShuffle()} aria-label={t('player.shuffle')} aria-pressed={shuffleOn}>
			<HugeiconsIcon icon={ShuffleIcon} class="h-6 w-6 {shuffleOn ? 'text-primary' : 'text-muted-foreground'}" />
		</button>
		<button class="{round} size-14" onclick={() => api.prevTrack()} aria-label={t('player.previous')}>
			<HugeiconsIcon icon={PreviousIcon} class="h-8 w-8" />
		</button>
		<button
			class="inline-flex size-[4.5rem] items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg transition-transform active:scale-95"
			onclick={() => api.togglePause()}
			aria-label={playback.paused ? t('player.play') : t('player.pause')}
		>
			<HugeiconsIcon icon={PauseIcon} altIcon={PlayIcon} showAlt={playback.paused} class="h-9 w-9" />
		</button>
		<button class="{round} size-14" onclick={() => api.nextTrack()} aria-label={t('player.next')}>
			<HugeiconsIcon icon={NextIcon} class="h-8 w-8" />
		</button>
		<button
			class={round}
			onclick={cycleRepeat}
			aria-label={t('player.repeat_state', {
				state: repeat === 'off' ? t('player.repeat_off') : repeat === 'one' ? t('player.repeat_one') : t('player.repeat_all')
			})}
			aria-pressed={repeat !== 'off'}
		>
			<HugeiconsIcon
				icon={RepeatIcon}
				altIcon={RepeatOne01Icon}
				showAlt={repeat === 'one'}
				class="h-6 w-6 {repeat !== 'off' ? 'text-primary' : 'text-muted-foreground'}"
			/>
		</button>
	</div>

	<!-- Sheets and phone extras. -->
	<div
		class="flex shrink-0 justify-around gap-1 px-3 pt-3"
		style="padding-bottom: calc(0.75rem + var(--safe-bottom, 0px))"
	>
		<button class={pill} onclick={() => (mobile.sheet = 'queue')}>
			<HugeiconsIcon icon={Queue01Icon} class="h-5 w-5 text-foreground" />
			{t('mobile.up_next')}
		</button>
		<button class={pill} onclick={() => (mobile.sheet = 'lyrics')}>
			<HugeiconsIcon icon={Mic01Icon} class="h-5 w-5 text-foreground" />
			{t('player.lyrics')}
		</button>
		{#if !local}
			<button class={pill} onclick={() => (mobile.sheet = 'comments')}>
				<HugeiconsIcon icon={Comment01Icon} class="h-5 w-5 text-foreground" />
				{t('player.comments')}
			</button>
		{/if}
		<button class={pill} onclick={() => (mobile.sheet = 'sleep')}>
			<HugeiconsIcon icon={Moon02Icon} class="h-5 w-5 {sleepOn ? 'text-primary' : 'text-foreground'}" />
			{t('mobile.sleep')}
		</button>
		<!-- iOS has no system equalizer an app can open. -->
		{#if isAndroid}
			<button class={pill} onclick={eq}>
				<HugeiconsIcon icon={SlidersHorizontalIcon} class="h-5 w-5 text-foreground" />
				{t('mobile.equalizer')}
			</button>
		{/if}
		{#if !local}
			<button class={pill} onclick={shareNow}>
				<HugeiconsIcon icon={Share08Icon} class="h-5 w-5 text-foreground" />
				{t('mobile.share')}
			</button>
		{/if}
	</div>
</div>

{#if mobile.sheet === 'queue'}
	<MobileSheet title={t('mobile.up_next')} tall onClose={() => (mobile.sheet = null)}>
		<QueueList />
	</MobileSheet>
{:else if mobile.sheet === 'lyrics'}
	<MobileSheet title={t('player.lyrics')} tall onClose={() => (mobile.sheet = null)}>
		<LyricsView />
	</MobileSheet>
{:else if mobile.sheet === 'comments'}
	<MobileSheet title={t('player.comments')} tall onClose={() => (mobile.sheet = null)}>
		<CommentsView />
	</MobileSheet>
{:else if mobile.sheet === 'sleep'}
	<MobileSheet title={t('mobile.sleep_timer')} onClose={() => (mobile.sheet = null)}>
		<SleepTimer onDone={() => (mobile.sheet = null)} />
	</MobileSheet>
{/if}
