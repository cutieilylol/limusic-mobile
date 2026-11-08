/**
 * The phone builds' shell state. The SPA is the same one the desktop app runs; on a phone the
 * root layout swaps the window chrome (titlebar, sidebar, player bar, floating panels) for a
 * bottom navigation bar, a mini player, a full-screen player and bottom sheets. Everything below
 * the shell (pages, cards, rows, menus, dialogs) is shared.
 */
import { browser } from '$app/environment';
import { invoke } from '@tauri-apps/api/core';

/** Android WebView. The Rust side is the Android build too whenever this is true. */
export const isAndroid = browser && /Android/i.test(navigator.userAgent);
/** iOS WKWebView. iPadOS reports itself as a Mac; its touch points are what give it away. */
export const isIOS =
	browser &&
	(/iPhone|iPad|iPod/.test(navigator.userAgent) ||
		(/Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1));
/** Either phone: the phone shell and its layouts. What only one has goes on the two above. */
export const isPhone = isAndroid || isIOS;

export type Sheet = 'queue' | 'lyrics' | 'comments' | 'sleep' | null;

export const mobile = $state({
	/** The bottom sheet open over the full-screen player. */
	sheet: null as Sheet,
	/** Sleep timer as the Rust side reports it: an absolute deadline (ms since epoch), or "at the
	 *  end of this track", or off. */
	sleep: { at: null as number | null, endOfTrack: false }
});

if (isPhone) {
	document.documentElement.classList.add('phone', 'touch');
}
if (isAndroid) {
	// The system bar sizes arrive from MainActivity.kt; a document that starts over (a reload, a
	// renderer restart) asks for them again instead of drawing under the bars.
	setTimeout(() => nativeAction('insets').catch(() => {}), 0);
}
if (isIOS) {
	document.documentElement.classList.add('ios');
	// Under the notch and the home bar (`:root.ios` in layout.css pads for them), and no zooming
	// in on a text field the moment it takes focus.
	document
		.querySelector('meta[name=viewport]')
		?.setAttribute(
			'content',
			'width=device-width, initial-scale=1, maximum-scale=1, viewport-fit=cover'
		);
	// WebKit on iOS never turns a long press into `contextmenu` the way Android's WebView does.
	longPress(document.documentElement);
}

/** Ask the native side for something only a phone has. Thin wrappers over `native_action`. */
export function nativeAction(action: string, args: Record<string, unknown> = {}): Promise<void> {
	return invoke('native_action', { action, args }).then(() => undefined);
}

export const share = (text: string, title?: string) => nativeAction('share', { text, title });
export const openEqualizer = () => nativeAction('equalizer');
export const keepScreenOn = (on: boolean) => nativeAction('keepScreenOn', { on });

/** A short buzz for a gesture that did something (a swipe to skip, a long press). */
export function haptic(ms = 12) {
	if (isAndroid) navigator.vibrate?.(ms);
	else if (isIOS) nativeAction('haptic').catch(() => {});
}

// ---- back button --------------------------------------------------------------------------------

type BackHandler = () => boolean;
const backHandlers: BackHandler[] = [];

/** Register a layer that wants the back button while it is up (a sheet, the player). The most
 *  recent one is asked first; returning true means it closed itself. */
export function onBack(fn: BackHandler): () => void {
	backHandlers.push(fn);
	return () => {
		const i = backHandlers.lastIndexOf(fn);
		if (i >= 0) backHandlers.splice(i, 1);
	};
}

/** What MainActivity.kt asks on every back press. `true`: handled here. `false`: the activity
 *  goes back in the webview's history, or to the home screen when there is none. */
function handleBack(): boolean {
	// A dialog or a menu over everything: bits-ui closes those on Escape.
	const overlay = document.querySelector(
		'[data-slot="dialog-content"][data-state="open"], [data-slot="alert-dialog-content"][data-state="open"], [role="menu"][data-state="open"], [data-popover-content][data-state="open"], [data-slot="popover-content"][data-state="open"]'
	);
	// An item's action sheet (fitMenu): its backdrop is the element right before it, and clicking
	// that is how every one of them closes. Never take the sheet out by hand: Svelte owns those
	// nodes and has not flushed the close yet, and a block that loses one renders nowhere ever
	// after (the menu stops opening until the app restarts). One with no backdrop gets an Escape.
	const sheet = document.querySelector<HTMLElement>('[data-sheet]');
	if (sheet) {
		const backdrop = sheet.previousElementSibling as HTMLElement | null;
		if (backdrop?.matches('.fixed.inset-0')) backdrop.click();
		else sheet.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		return true;
	}
	if (overlay) {
		const ev = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
		(overlay as HTMLElement).dispatchEvent(ev);
		document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		return true;
	}
	for (let i = backHandlers.length - 1; i >= 0; i--) {
		if (backHandlers[i]()) return true;
	}
	return false;
}

if (isAndroid) {
	(window as unknown as { __limusicBack: () => boolean }).__limusicBack = handleBack;
}

// ---- swipe gestures -------------------------------------------------------------------------------

export type SwipeDir = 'left' | 'right' | 'up' | 'down';

/**
 * A Svelte attachment for one-finger swipes. `onMove` gets the live offset (for things that follow
 * the finger), `onSwipe` the direction once a swipe clears `min` px, `onEnd` fires either way.
 * Taps and vertical scrolls inside the element are left alone.
 */
export function swipe(opts: {
	onSwipe: (dir: SwipeDir) => void;
	onMove?: (dx: number, dy: number) => void;
	onEnd?: () => void;
	axis?: 'x' | 'y' | 'both';
	min?: number;
}) {
	return (el: HTMLElement) => {
		const min = opts.min ?? 60;
		let x0 = 0;
		let y0 = 0;
		let t0 = 0;
		let tracking = false;
		let locked: 'x' | 'y' | null = null;
		const down = (e: TouchEvent) => {
			if (e.touches.length !== 1) return;
			x0 = e.touches[0].clientX;
			y0 = e.touches[0].clientY;
			t0 = performance.now();
			tracking = true;
			locked = null;
		};
		const move = (e: TouchEvent) => {
			if (!tracking) return;
			const dx = e.touches[0].clientX - x0;
			const dy = e.touches[0].clientY - y0;
			if (!locked && Math.hypot(dx, dy) > 10) locked = Math.abs(dx) > Math.abs(dy) ? 'x' : 'y';
			const axis = opts.axis ?? 'both';
			if (locked && (axis === 'both' || axis === locked)) {
				opts.onMove?.(locked === 'x' ? dx : 0, locked === 'y' ? dy : 0);
				if (locked === 'x' && e.cancelable) e.preventDefault();
			}
		};
		const up = (e: TouchEvent) => {
			if (!tracking) return;
			tracking = false;
			const t = e.changedTouches[0];
			const dx = t.clientX - x0;
			const dy = t.clientY - y0;
			const fast = performance.now() - t0 < 300;
			const axis = opts.axis ?? 'both';
			const need = fast ? min / 2 : min;
			if (locked === 'x' && axis !== 'y' && Math.abs(dx) > need) {
				opts.onSwipe(dx < 0 ? 'left' : 'right');
			} else if (locked === 'y' && axis !== 'x' && Math.abs(dy) > need) {
				opts.onSwipe(dy < 0 ? 'up' : 'down');
			}
			opts.onEnd?.();
		};
		el.addEventListener('touchstart', down, { passive: true });
		el.addEventListener('touchmove', move, { passive: false });
		el.addEventListener('touchend', up, { passive: true });
		el.addEventListener('touchcancel', up, { passive: true });
		return () => {
			el.removeEventListener('touchstart', down);
			el.removeEventListener('touchmove', move);
			el.removeEventListener('touchend', up);
			el.removeEventListener('touchcancel', up);
		};
	};
}

/** Long press for a touch screen: the phone's stand-in for right-click. Opens whatever the element's
 *  context menu would (the desktop `data-ctx` path dispatches `contextmenu`). */
export function longPress(el: HTMLElement) {
	if (!isPhone) return;
	let timer: ReturnType<typeof setTimeout> | null = null;
	let x = 0;
	let y = 0;
	// The press has fired and the finger is still down: the tap it ends in must not also play the
	// row under it. Dropped shortly after the finger lifts, so the menu's own first tap gets through.
	let swallowing = false;
	const swallow = (e: Event) => {
		e.preventDefault();
		e.stopPropagation();
	};
	const stopSwallowing = () => {
		swallowing = false;
		window.removeEventListener('click', swallow, true);
	};
	const cancel = () => {
		if (timer) clearTimeout(timer);
		timer = null;
		if (swallowing) setTimeout(stopSwallowing, 350);
	};
	const down = (e: TouchEvent) => {
		if (e.touches.length !== 1) return;
		x = e.touches[0].clientX;
		y = e.touches[0].clientY;
		const target = e.target as HTMLElement;
		cancel();
		if (!target.closest?.('[data-ctx]')) return;
		timer = setTimeout(() => {
			timer = null;
			haptic(20);
			swallowing = true;
			window.addEventListener('click', swallow, true);
			target.dispatchEvent(
				new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: x, clientY: y })
			);
		}, 480);
	};
	const move = (e: TouchEvent) => {
		const t = e.touches[0];
		if (Math.hypot(t.clientX - x, t.clientY - y) > 10) cancel();
	};
	el.addEventListener('touchstart', down, { passive: true });
	el.addEventListener('touchmove', move, { passive: true });
	el.addEventListener('touchend', cancel, { passive: true });
	el.addEventListener('touchcancel', cancel, { passive: true });
	return () => {
		cancel();
		el.removeEventListener('touchstart', down);
		el.removeEventListener('touchmove', move);
		el.removeEventListener('touchend', cancel);
		el.removeEventListener('touchcancel', cancel);
	};
}

// ---- sleep timer ----------------------------------------------------------------------------------

type SleepState = { at: number | null; endOfTrack: boolean };

export async function setSleepTimer(minutes: number | null, endOfTrack = false) {
	const s = await invoke<SleepState>('set_sleep_timer', { minutes, endOfTrack });
	mobile.sleep = s;
}

/** Pick the timer's state up from Rust, and follow it (it clears itself when it fires). */
export async function initSleepTimer(): Promise<() => void> {
	const { listen } = await import('@tauri-apps/api/event');
	mobile.sleep = await invoke<SleepState>('get_sleep_timer');
	return listen<SleepState>('sleep-timer', (e) => (mobile.sleep = e.payload));
}

// ---- permissions ------------------------------------------------------------------------------

/** Ask for the phone's "music and audio" permission and resolve with the Music folder's path, or
 *  null if the user said no. */
export async function requestMusicFolder(): Promise<string | null> {
	const { listen } = await import('@tauri-apps/api/event');
	return new Promise((resolve) => {
		let un: (() => void) | undefined;
		listen<{ ev: string; granted: boolean; music: string }>('android-event', (e) => {
			if (e.payload.ev !== 'audioPermission') return;
			un?.();
			resolve(e.payload.granted ? e.payload.music : null);
		}).then((u) => {
			un = u;
			nativeAction('audioPermission').catch(() => resolve(null));
		});
	});
}
