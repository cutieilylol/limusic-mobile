/**
 * Offline downloads (src-tauri/src/downloads.rs): what is on the device, what is still coming,
 * and the actions the menus and the Downloads page call. Rust owns the list; this mirrors it
 * from the `downloads-changed` event.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { SongItem } from './api';
import { toast } from './player.svelte';
import { t } from './i18n.svelte';

export type DownloadEntry = {
	song: SongItem;
	file: string;
	bytes: number;
	addedAt: number;
};

type Status = {
	done: DownloadEntry[];
	queued: SongItem[];
	progress: [number, number] | null;
};

export const downloads = $state({
	done: [] as DownloadEntry[],
	queued: [] as SongItem[],
	progress: null as [number, number] | null,
	ids: new Set<string>(),
	ready: false
});

function apply(s: Status) {
	downloads.done = s.done;
	downloads.queued = s.queued;
	downloads.progress = s.progress;
	downloads.ids = new Set(s.done.map((d) => d.song.video_id));
	downloads.ready = true;
}

let started = false;
export function initDownloads(): void {
	if (started) return;
	started = true;
	invoke<Status>('get_downloads').then(apply).catch(() => {});
	listen<Status>('downloads-changed', (e) => apply(e.payload));
	listen<{ done: number; total: number }>('download-progress', (e) => {
		downloads.progress = [e.payload.done, e.payload.total];
	});
	listen<{ title: string; error: string }>('download-failed', (e) =>
		toast.error(t('downloads.failed', { title: e.payload.title }))
	);
}

export const isDownloaded = (id: string) => downloads.ids.has(id);
export const isQueued = (id: string) => downloads.queued.some((s) => s.video_id === id);

export async function download(songs: SongItem[]) {
	initDownloads();
	const fresh = songs.filter((s) => !isDownloaded(s.video_id) && !isQueued(s.video_id));
	if (!fresh.length) return;
	await invoke('download_songs', { songs: fresh });
	toast.success(
		fresh.length === 1
			? t('downloads.started_one', { title: fresh[0].title })
			: t('downloads.started_many', { count: fresh.length })
	);
}

export async function removeDownloads(ids: string[]) {
	await invoke('remove_downloads', { videoIds: ids });
}

export const fmtBytes = (n: number) =>
	n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : `${Math.round(n / 1e3)} kB`;
