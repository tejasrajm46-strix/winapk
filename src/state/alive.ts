import { useEffect, useRef } from "react";

/**
 * A ref that is false once the component unmounted.
 *
 * Every owner below starts async reads; an answer that arrives after the window
 * is gone must not set state on a dead tree. Each owner keeps its own ref so no
 * one has to pass a lifetime around.
 */
export function useAlive() {
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  return alive;
}
