import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Api, validateConnection, type Connection } from '../../lib/api';
import { desktop, errorText } from '../../lib/desktop';

export function useServerSession() {
  const [connection, setConnection] = useState<Connection | null>(null);
  const [connecting, setConnecting] = useState(desktop.available());
  const [error, setError] = useState('');
  const attempt = useRef(0);
  const api = useMemo(() => connection ? new Api(connection) : null, [connection]);

  const connectDesktop = useCallback(async () => {
    const generation = ++attempt.current;
    setConnecting(true); setError('');
    try {
      const next = validateConnection(await desktop.connection());
      if (generation === attempt.current) setConnection(next);
    } catch (cause) {
      if (generation === attempt.current) setError(errorText(cause));
    } finally {
      if (generation === attempt.current) setConnecting(false);
    }
  }, []);

  useEffect(() => {
    if (desktop.available()) void connectDesktop();
    return () => { attempt.current++; };
  }, [connectDesktop]);

  function connect(next: Connection) {
    const validated = validateConnection(next);
    attempt.current++;
    setConnecting(false); setError(''); setConnection(validated);
  }

  return { api, connecting, error, connectDesktop, connect };
}
