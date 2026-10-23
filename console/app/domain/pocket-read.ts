import { formatDateClock } from "bmo-ui/app/domain/time";
import { ref } from "vue";

import type { Ref } from "vue";

// One list a view reads: what it holds once read, whether it is being read, and why the last read failed.
export interface Reading<T> {
  items: Ref<T[]>;
  loading: Ref<boolean>;
  failed: Ref<string | null>;
  read: () => Promise<void>;
}

function said(error: unknown): string {
  const answered = (error as { data?: { error?: unknown } } | null)?.data?.error;

  if (typeof answered === "string") return answered;

  return error instanceof Error ? error.message : String(error);
}

/** A list read by `ask` each time `read` is called, its last answer kept until the next one lands. */
export function reading<T>(ask: () => Promise<T[]>): Reading<T> {
  const items = ref<T[]>([]) as Ref<T[]>;
  const loading = ref(false);
  const failed = ref<string | null>(null);

  async function read(): Promise<void> {
    loading.value = items.value.length === 0;

    try {
      items.value = await ask();
      failed.value = null;
    } catch (error) {
      failed.value = said(error);
    } finally {
      loading.value = false;
    }
  }

  return { items, loading, failed, read };
}

/** A time in epoch milliseconds as the layer writes a date and a clock. */
export function stamp(ms: number): string {
  return formatDateClock(Math.floor(ms / 1000));
}
