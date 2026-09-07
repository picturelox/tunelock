import { useEffect } from 'react';
import { sessionService } from './sessionService';
import { useSessionStore } from './sessionStore';

export default function SessionRuntime() {
  const engineStatus = useSessionStore((state) => state.engine.status);

  useEffect(() => {
    void sessionService.initialize().catch(() => {
      // The store owns the visible initialization error and retry state.
    });
  }, []);

  useEffect(() => {
    if (engineStatus !== 'ready') return;

    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      if (!active) return;
      try {
        await sessionService.pollMeters();
      } catch {
        // A transient telemetry failure does not destroy the live session.
      }
      if (active) timer = setTimeout(poll, 50);
    };

    timer = setTimeout(poll, 50);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [engineStatus]);

  return null;
}

