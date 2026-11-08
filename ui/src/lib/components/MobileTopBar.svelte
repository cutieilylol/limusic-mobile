<script lang="ts">
	import { afterNavigate } from '$app/navigation';
	import { page } from '$app/state';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ArrowLeft01Icon,
		Settings02Icon,
		Link04Icon,
		UserGroup02Icon
	} from '@hugeicons/core-free-icons';
	import AccountMenu from './AccountMenu.svelte';
	import { appIcon } from '$lib/appicon.svelte';
	import { ui } from '$lib/player.svelte';
	import { lt } from '$lib/lt.svelte';
	import { t } from '$lib/i18n.svelte';

	// History depth, the same bookkeeping the desktop titlebar does for its back button: shown only
	// when there is somewhere to go back to. (The phone's own back button works either way.)
	let depth = $state(0);
	afterNavigate((nav) => {
		if (nav.type === 'enter') depth = 0;
		else if (nav.delta !== undefined) depth = Math.max(0, depth + nav.delta);
		else depth += 1;
	});

	// The four bottom-nav destinations are roots: they get the app's name, never a back arrow.
	const root = $derived(['/', '/search', '/library', '/stats'].includes(page.url.pathname));

	const btn =
		'inline-flex size-10 items-center justify-center rounded-full text-muted-foreground active:bg-muted';
</script>

<!-- No backdrop-blur: nothing scrolls under this bar (the page scrolls below it), and a filter
     here would make the bar the box `position: fixed` resolves against, which pinned the account
     menu's sheet to the bar instead of the bottom of the screen. -->
<header
	class="relative z-40 flex shrink-0 items-center gap-1 bg-background px-2"
	style="padding-top: var(--safe-top, env(safe-area-inset-top, 0px)); min-height: calc(3.25rem + var(--safe-top, env(safe-area-inset-top, 0px)))"
>
	{#if depth > 0 && !root}
		<button class={btn} onclick={() => history.back()} aria-label={t('common.back')}>
			<HugeiconsIcon icon={ArrowLeft01Icon} class="h-5 w-5" />
		</button>
	{:else}
		<div class="flex items-center gap-2 pl-2">
			{#if appIcon.src}
				<img src={appIcon.src} alt="" class="h-6 w-6 rounded-md" />
			{/if}
			<span class="text-lg font-semibold tracking-tight">Limusic</span>
		</div>
	{/if}
	<div class="flex-1"></div>
	<button
		class="{btn} {lt.role !== 'none' ? 'text-primary' : ''}"
		onclick={() => (ui.ltOpen = true)}
		aria-label={t('nav.listen_together')}
	>
		<HugeiconsIcon icon={UserGroup02Icon} class="h-5 w-5" />
	</button>
	<button class={btn} onclick={() => (ui.linkOpen = true)} aria-label={t('dialogs.link.title')}>
		<HugeiconsIcon icon={Link04Icon} class="h-5 w-5" />
	</button>
	<button class={btn} onclick={() => (ui.settingsOpen = true)} aria-label={t('nav.settings')}>
		<HugeiconsIcon icon={Settings02Icon} class="h-5 w-5" />
	</button>
	<AccountMenu />
</header>
