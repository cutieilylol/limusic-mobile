<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { MusicNote01Icon, ListRestartIcon, PinIcon } from '@hugeicons/core-free-icons';
	import { ON_REPEAT_ID, type BrowseItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { openItem } from '$lib/browse';
	import ItemMenu from './ItemMenu.svelte';
	import ExplicitIcon from './ExplicitIcon.svelte';

	// A library entry as a list row, the phone's version of `MediaCard`: cover, title, subtitle and
	// ⋮, the layout YouTube Music and Spotify use for a library. Same click (open it), same menu, and
	// a long press opens that menu too (`data-ctx`).
	let { item, pinned = false }: { item: BrowseItem; pinned?: boolean } = $props();

	const round = $derived(item.kind === 'artist');
	const onRepeat = $derived(item.id === ON_REPEAT_ID);
	let failed = $state(false);
	$effect(() => {
		item.thumbnail;
		failed = false;
	});
</script>

<div class="group relative flex items-center" data-ctx>
	<button
		type="button"
		class="flex min-w-0 flex-1 items-center gap-3 rounded-xl py-1.5 pl-1 pr-2 text-left active:bg-muted/60"
		onclick={() => openItem(item)}
	>
		{#if onRepeat}
			<span class="flex h-14 w-14 shrink-0 items-center justify-center rounded-lg bg-primary/15 text-primary">
				<HugeiconsIcon icon={ListRestartIcon} class="h-6 w-6" />
			</span>
		{:else if item.thumbnail && !failed}
			<img
				src={thumb(item.thumbnail, 120)}
				alt=""
				loading="lazy"
				onerror={() => (failed = true)}
				style="max-width:none"
				class="h-14 w-14 shrink-0 bg-muted object-cover {round ? 'rounded-full' : 'rounded-lg'}"
			/>
		{:else}
			<span
				class="flex h-14 w-14 shrink-0 items-center justify-center bg-muted text-muted-foreground/50 {round
					? 'rounded-full'
					: 'rounded-lg'}"
			>
				<HugeiconsIcon icon={MusicNote01Icon} class="h-5 w-5" />
			</span>
		{/if}
		<span class="min-w-0 flex-1">
			<span class="block truncate text-[0.95rem] font-medium">{item.title}</span>
			<span class="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
				{#if pinned}<HugeiconsIcon icon={PinIcon} class="h-3 w-3 shrink-0 text-primary" />{/if}
				{#if item.explicit}<ExplicitIcon class="h-3 w-3 shrink-0" />{/if}
				<span class="truncate">{item.subtitle ?? ''}</span>
			</span>
		</span>
	</button>
	<ItemMenu
		{item}
		triggerClass="flex h-10 w-10 shrink-0 items-center justify-center rounded-full text-muted-foreground active:bg-muted"
	/>
</div>
