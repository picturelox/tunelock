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

// Provisional jog sensitivity. The S3 reports an 8-bit distance-tick delta per
// ~140 Hz report; this constant maps that delta to a signed playback rate
// (1.0 = normal forward speed). Needs calibration against the physical wheel.
const JOG_TICKS_PER_REPORT_AT_1X = 25;

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
      void sessionService.jogTouch(deck, action.pressed).catch(console.error);
      break;
    }

    case 'jog': {
      const deck = S3_DECK_TO_DECK_ID[action.deck];
      const rate = action.delta / JOG_TICKS_PER_REPORT_AT_1X;
      void sessionService.jogRate(deck, rate).catch(console.error);
      break;
    }

    case 'fader': {
      // Fader index -> control mapping is not yet reverse-engineered; the
      // semantic action is emitted but intentionally not applied yet.
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
  } catch (error) {
    console.warn('[s3] failed to start reader:', error);
  }

  return () => {
    unlistenAction();
    unlistenStatus();
    unsubscribeStore();
  };
}
