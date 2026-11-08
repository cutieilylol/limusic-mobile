<script lang="ts">
	import { onMount } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Moon02Icon, Cancel01Icon, MusicNote01Icon } from '@hugeicons/core-free-icons';
	import { mobile, setSleepTimer } from '$lib/mobile.svelte';
	import { t } from '$lib/i18n.svelte';

	// Pause after a while, or at the end of this song: the sleep timer every phone music app has.
	// The countdown itself runs in Rust (phone.rs), which keeps time with the screen off.
	let { onDone }: { onDone: () => void } = $props();

	const PRESETS = [5, 10, 15, 30, 45, 60, 90];
	let now = $state(Date.now());
	onMount(() => {
		const id = setInterval(() => (now = Date.now()), 1000);
		return () => clearInterval(id);
	});

	const left = $derived(mobile.sleep.at ? Math.max(0, mobile.sleep.at - now) : 0);
	const fmt = (ms: number) => {
		const s = Math.ceil(ms / 1000);
		const h = Math.floor(s / 3600);
		const m = Math.floor((s % 3600) / 60);
		const ss = (s % 60).toString().padStart(2, '0');
		return h ? `${h}:${m.toString().padStart(2, '0')}:${ss}` : `${m}:${ss}`;
	};

	async function pick(minutes: number | null, endOfTrack = false) {
		await setSleepTimer(minutes, endOfTrack);
		onDone();
	}

	const row =
		'flex w-full items-center gap-3 rounded-xl px-3 py-3 text-left text-sm active:bg-muted';
</script>

<div class="flex flex-col gap-1 pb-2">
	{#if mobile.sleep.at || mobile.sleep.endOfTrack}
		<div class="mx-1 mb-2 flex items-center gap-3 rounded-2xl bg-primary/10 px-4 py-3 text-primary">
			<HugeiconsIcon icon={Moon02Icon} class="h-5 w-5" />
			<span class="flex-1 text-sm font-medium">
				{mobile.sleep.endOfTrack ? t('mobile.sleep_end_of_track_on') : t('mobile.sleep_left', { time: fmt(left) })}
			</span>
			<button class="rounded-full px-3 py-1 text-sm font-medium active:bg-primary/20" onclick={() => pick(null)}>
				{t('mobile.sleep_off')}
			</button>
		</div>
	{/if}
	{#each PRESETS as m (m)}
		<button class={row} onclick={() => pick(m)}>
			<span class="w-5"></span>
			{m < 60 ? t('mobile.minutes', { n: m }) : m === 60 ? t('mobile.one_hour') : t('mobile.minutes', { n: m })}
		</button>
	{/each}
	<button class={row} onclick={() => pick(null, true)}>
		<HugeiconsIcon icon={MusicNote01Icon} class="h-5 w-5 text-muted-foreground" />
		{t('mobile.sleep_end_of_track')}
	</button>
	{#if mobile.sleep.at || mobile.sleep.endOfTrack}
		<button class="{row} text-destructive" onclick={() => pick(null)}>
			<HugeiconsIcon icon={Cancel01Icon} class="h-5 w-5" />
			{t('mobile.sleep_turn_off')}
		</button>
	{/if}
</div>
