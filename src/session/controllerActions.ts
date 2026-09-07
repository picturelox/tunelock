import {
  s3Start,
  s3SetLeds,
  onS3Action,
  onS3Status,
  type S3Action,
  type S3Deck,
  type S3LedState,
} from '../lib/tauri';
import { sessionService } from './sessionService';
import { useSessionStore } from './sessionStore';
import type { DeckId } from './types';

// TL-07 shared action model: maps the hardware-neutral S3Action vocabulary
// emitted by the Rust adapter onto the same sessionService commands used by
// mouse and keyboard. This is the single place hardware intent becomes a
// session command.

const S3_DECK_TO_DECK_ID: Record<S3Deck, DeckId> = {
  a: 'A',
  b: 'B',
};

// The S3 jog timecode advances at 400 kHz. At 33 1/3 RPM, one 768-tick
// rotation takes 1.8 seconds, so this is the device tick/time ratio at 1x.
const JOG_RATIO_AT_1X = 768 / 720_000;
const JOG_IDLE_MS = 40;
const JOG_STALE_MS = 20_000;
const touched: Record<S3Deck, boolean> = { a: false, b: false };
const lastJogAt: Record<S3Deck, number> = { a: 0, b: 0 };
const jogIdleTimers: Partial<Record<S3Deck, ReturnType<typeof setTimeout>>> = {};

function otherDeck(deck: DeckId): DeckId {
  return deck === 'A' ? 'B' : 'A';
}

export function dispatchS3Action(action: S3Action): void {
  switch (action.type) {
    case 'play': {
      if (!action.pressed) return;
      const deck = S3_DECK_TO_DECK_ID[action.deck];
      const current = useSessionStore.getState().decks[deck].acknowledgedTransport;
      const next = current === 'playing' ? 'paused' : 'playing';
      void sessionService.setTransport(deck, next).catch(console.error);
      break;
    }

    case 'cue': {
      if (!action.pressed) return;
      const deck = S3_DECK_TO_DECK_ID[action.deck];
      // Provisional: there is no dedicated transport cue-point model yet, so
      // Cue jumps to the start of the track and pauses.
      void sessionService.setTransport(deck, 'paused').catch(console.error);
      void sessionService.seek(deck, 0).catch(console.error);
      break;
    }

    case 'sync': {
      if (!action.pressed) return;
      const deck = S3_DECK_TO_DECK_ID[action.deck];
      // beatSync(leader, follower): the pressed deck follows the other deck.
      void sessionService.beatSync(otherDeck(deck), deck).catch(console.error);
      break;
    }

    case 'hotCue': {
      if (!action.pressed) return;
      const deck = S3_DECK_TO_DECK_ID[action.deck];
      void sessionService.jumpHotCue(deck, action.slot).catch(console.error);
      break;
    }

    case 'touch': {
      const deck = S3_DECK_TO_DECK_ID[action.deck];
      touched[action.deck] = action.pressed;
      if (!action.pressed) {
        const timer = jogIdleTimers[action.deck];
        if (timer !== undefined) clearTimeout(timer);
        delete jogIdleTimers[action.deck];
      }
      void sessionService.jogTouch(deck, action.pressed).catch(console.error);
      break;
    }

    case 'jog': {
      const deck = S3_DECK_TO_DECK_ID[action.deck];
      const now = Date.now();
      const stale = now - lastJogAt[action.deck] > JOG_STALE_MS;
      lastJogAt[action.deck] = now;

      if (touched[action.deck]) {
        if (!stale) {
          const rate = (action.tickDelta / action.timeDelta) / JOG_RATIO_AT_1X;
          void sessionService.jogRate(deck, rate).catch(console.error);
        }
        const previousTimer = jogIdleTimers[action.deck];
        if (previousTimer !== undefined) clearTimeout(previousTimer);
        jogIdleTimers[action.deck] = setTimeout(() => {
          void sessionService.jogRate(deck, 0).catch(console.error);
          delete jogIdleTimers[action.deck];
        }, JOG_IDLE_MS);
      } else {
        // Untouched wheels nudge by physical platter distance. One rotation is
        // 1.8 seconds of track at nominal speed; convert that distance to beats.
        const reportedBpm = useSessionStore.getState().meters
          ?.players[S3_DECK_TO_DECK_ID[action.deck] === 'A' ? 0 : 1]
          ?.sourceBpm;
        const sourceBpm = reportedBpm && reportedBpm > 0 ? reportedBpm : 120;
        const beats = action.tickDelta * (1.8 * sourceBpm / 60) / 768;
        void sessionService.nudge(deck, beats).catch(console.error);
      }
      break;
    }

    case 'control': {
      // The adapter now emits named controls. Mixer/EQ/gain routing stays
      // intentionally deferred to TL-08, where ranges and gain staging are
      // specified and measured. Tempo is likewise not applied until its
      // supported hardware range and soft-takeover behavior are contracted.
      break;
    }
  }
}

/**
 * Derive the S3 button backlight state from the current session transport.
 *
 * Play mirrors `playing`; Cue mirrors `paused` (provisional — TuneLock has no
 * transport cue-point model yet); Sync is left off until a persistent sync
 * state exists.
 */
function buildLedState(): S3LedState {
  const decks = useSessionStore.getState().decks;
  return {
    playA: decks.A.acknowledgedTransport === 'playing',
    playB: decks.B.acknowledgedTransport === 'playing',
    cueA: decks.A.acknowledgedTransport === 'paused',
    cueB: decks.B.acknowledgedTransport === 'paused',
    syncA: false,
    syncB: false,
  };
}

function syncS3Leds(): void {
  void s3SetLeds(buildLedState()).catch((error) => {
    console.warn('[s3] LED write failed:', error);
  });
}

/**
 * Start the S3 hardware adapter: subscribe to action/status events, ask the
 * Rust side to open the device, and mirror transport state onto the LEDs.
 * Returns an unsubscribe function.
 */
export async function startS3Controller(): Promise<() => void> {
  const unlistenAction = await onS3Action(dispatchS3Action);
  const unlistenStatus = await onS3Status((status) => {
    console.info(
      '[s3]',
      status.connected ? 'connected' : 'disconnected',
      status.error ?? '',
    );
    if (status.connected) syncS3Leds();
  });

  const unsubscribeStore = useSessionStore.subscribe((state, previous) => {
    const a = state.decks.A.acknowledgedTransport;
    const b = state.decks.B.acknowledgedTransport;
    if (
      a !== previous.decks.A.acknowledgedTransport
      || b !== previous.decks.B.acknowledgedTransport
    ) {
      syncS3Leds();
    }
  });

  try {
    await s3Start();
    // Also push once after an idempotent start. This covers a listener remount
    // after the long-lived Rust reader has already emitted its connected event.
    syncS3Leds();
  } catch (error) {
    console.warn('[s3] failed to start reader:', error);
  }

  return () => {
    unlistenAction();
    unlistenStatus();
    unsubscribeStore();
    for (const s3Deck of ['a', 'b'] as const) {
      const timer = jogIdleTimers[s3Deck];
      if (timer !== undefined) clearTimeout(timer);
      delete jogIdleTimers[s3Deck];
      lastJogAt[s3Deck] = 0;
      if (touched[s3Deck]) {
        touched[s3Deck] = false;
        void sessionService.jogTouch(S3_DECK_TO_DECK_ID[s3Deck], false)
          .catch(console.error);
      }
    }
  };
}
