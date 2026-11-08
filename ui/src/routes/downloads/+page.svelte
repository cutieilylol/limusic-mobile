<script lang="ts">
	// Songs saved on the phone (downloads.rs), playable with no connection. Newest first, with
	// whatever is still downloading above them.
	import { onMount } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Download04Icon, PlayIcon, ShuffleIcon, Delete02Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import * as api from '$lib/api';
	import { openAddToPlaylist, openPlayer, playback } from '$lib/player.svelte';
	import { downloads, fmtBytes, initDownloads, removeDownloads } from '$lib/downloads.svelte';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';

	onMount(initDownloads);

	const songs = $derived(downloads.done.map((d) => d.song));
	const size = $derived(downloads.done.reduce((n, d) => n + d.bytes, 0));
	const nowId = $derived(playback.now?.videoId);
	const inFlight = $derived(downloads.queued[0]);
	const pct = $derived(
		downloads.progress && downloads.progress[1]
			? Math.round((downloads.progress[0] / downloads.progress[1]) * 100)
			: 0
	);

	function play(start: number | null, shuffle = false) {
		if (!songs.length) return;
		openPlayer();
		api.playPlaylist(songs, start, undefined, t('downloads.title'), shuffle);
	}

	function removeAll() {
		if (!songs.length) return;
		if (!confirm(t('downloads.remove_all_confirm', { count: songs.length }))) return;
		removeDownloads(songs.map((s) => s.video_id));
	}
</script>

<div class="px-4 pb-6 pt-2">
	<div class="mb-4 flex items-end gap-4">
		<div class="flex h-24 w-24 shrink-0 items-center justify-center rounded-2xl bg-primary/15 text-primary">
			<HugeiconsIcon icon={Download04Icon} class="h-10 w-10" />
		</div>
		<div class="min-w-0">
			<h1 class="font-heading text-3xl font-bold tracking-tight">{t('downloads.title')}</h1>
			<p class="mt-1 text-sm text-muted-foreground">
				{songs.length === 1
					? t('downloads.summary_one', { size: fmtBytes(size) })
					: t('downloads.summary', { count: songs.length, size: fmtBytes(size) })}
			</p>
		</div>
	</div>

	{#if songs.length}
		<div class="mb-4 flex items-center gap-2">
			<Button class="rounded-full" onclick={() => play(0)}>
				<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" />
				{t('common.play_all')}
			</Button>
			<Button variant="secondary" class="rounded-full" onclick={() => play(null, true)}>
				<HugeiconsIcon icon={ShuffleIcon} class="h-4 w-4" />
				{t('player.shuffle')}
			</Button>
			<div class="flex-1"></div>
			<Button variant="ghost" size="icon" onclick={removeAll} aria-label={t('downloads.remove_all')}>
				<HugeiconsIcon icon={Delete02Icon} class="h-5 w-5" />
			</Button>
		</div>
	{/if}

	{#if downloads.queued.length}
		<div class="mb-4 rounded-2xl border bg-card p-3">
			{#if inFlight}
				<div class="flex items-center gap-3">
					{#if inFlight.thumbnail}
						<img src={thumb(inFlight.thumbnail, 120)} alt="" class="h-11 w-11 shrink-0 rounded-lg object-cover" />
					{/if}
					<div class="min-w-0 flex-1">
						<p class="truncate text-sm font-medium">{inFlight.title}</p>
						<p class="truncate text-xs text-muted-foreground">
							{t('downloads.downloading')} {pct ? `${pct}%` : ''}
							{#if downloads.queued.length > 1}· {t('downloads.queued', { count: downloads.queued.length - 1 })}{/if}
						</p>
						<div class="mt-1.5 h-1 overflow-hidden rounded-full bg-muted">
							<div class="h-full bg-primary transition-[width]" style="width:{pct}%"></div>
						</div>
					</div>
				</div>
			{/if}
		</div>
	{/if}

	{#if songs.length}
		<div class="flex flex-col">
			{#each songs as song, i (song.video_id)}
				<TrackRow
					{song}
					lazy
					active={song.video_id === nowId}
					onplay={() => play(i)}
					onAdd={() => openAddToPlaylist(song)}
				/>
			{/each}
		</div>
	{:else if downloads.ready && !downloads.queued.length}
		<div class="flex flex-col items-center gap-2 py-16 text-center">
			<HugeiconsIcon icon={Download04Icon} class="h-10 w-10 text-muted-foreground/50" />
			<p class="text-sm font-medium">{t('downloads.empty')}</p>
			<p class="max-w-xs text-xs text-muted-foreground">{t('downloads.empty_hint')}</p>
		</div>
	{/if}
</div>
