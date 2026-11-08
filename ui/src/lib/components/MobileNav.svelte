<script lang="ts">
	import { page } from '$app/state';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Home01Icon, Search01Icon, LibraryIcon, Chart02Icon } from '@hugeicons/core-free-icons';
	import { t } from '$lib/i18n.svelte';

	// The phone's sidebar: the same four destinations, as a bottom bar under the thumb. Settings and
	// the account live in the top bar instead (MobileTopBar), like YouTube Music's avatar menu.
	const nav = $derived([
		{ href: '/', label: t('nav.home'), icon: Home01Icon },
		{ href: '/search', label: t('nav.search'), icon: Search01Icon },
		{ href: '/library', label: t('nav.library'), icon: LibraryIcon },
		{ href: '/stats', label: t('nav.stats'), icon: Chart02Icon }
	]);
	// iOS starts the app at the bare `tauri://localhost`, which has no path until the first
	// navigation gives it one.
	const path = $derived(page.url.pathname.replace(/\/index\.html$/, '') || '/');
	const isActive = (href: string) => (href === '/' ? path === '/' : path.startsWith(href));
</script>

<nav
	class="flex shrink-0 items-stretch justify-around border-t border-border/60 bg-card/95 backdrop-blur"
	style="padding-bottom: var(--safe-bottom, env(safe-area-inset-bottom, 0px))"
>
	{#each nav as n (n.href)}
		{@const on = isActive(n.href)}
		<a
			href={n.href}
			aria-current={on ? 'page' : undefined}
			class="flex min-h-14 flex-1 flex-col items-center justify-center gap-1 text-[11px] font-medium transition-colors {on
				? 'text-foreground'
				: 'text-muted-foreground'}"
		>
			<span
				class="flex h-7 w-14 items-center justify-center rounded-full transition-colors {on
					? 'bg-primary/15 text-primary'
					: ''}"
			>
				<HugeiconsIcon icon={n.icon} class="h-5 w-5" />
			</span>
			{n.label}
		</a>
	{/each}
</nav>
