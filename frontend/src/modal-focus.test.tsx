import render from 'preact-render-to-string';
import { describe, expect, it, vi } from 'vitest';
import { activateModalFocus } from './modal-focus';
import { Dialog } from './modal';

function nodeList<T extends Element>(items: T[]): NodeListOf<T> {
  return Object.assign([...items], {
    item: (index: number) => items[index] ?? null,
  }) as unknown as NodeListOf<T>;
}

class FakeDocument {
  activeElement: Element | null = null;
  modals: FakeElement[] = [];
  private readonly keydown = new Set<(event: KeyboardEvent) => void>();

  addEventListener(type: string, listener: (event: KeyboardEvent) => void): void {
    expect(type).toBe('keydown');
    this.keydown.add(listener);
  }

  removeEventListener(type: string, listener: (event: KeyboardEvent) => void): void {
    expect(type).toBe('keydown');
    this.keydown.delete(listener);
  }

  querySelectorAll<T extends Element>(): NodeListOf<T> {
    return nodeList(this.modals as unknown as T[]);
  }

  contains(node: Node | null): boolean {
    return node !== null && (node as unknown as FakeElement).isConnected;
  }

  dispatch(event: KeyboardEvent): void {
    for (const listener of this.keydown) listener(event);
  }
}

class FakeElement {
  readonly ownerDocument: Document;
  tabIndex = 0;
  isConnected = true;
  focusCount = 0;
  children: FakeElement[] = [];
  preferred: FakeElement | null = null;

  constructor(private readonly document: FakeDocument) {
    this.ownerDocument = document as unknown as Document;
  }

  focus(): void {
    this.focusCount += 1;
    this.document.activeElement = this as unknown as Element;
  }

  closest(): Element | null {
    return null;
  }

  querySelectorAll<T extends Element>(): NodeListOf<T> {
    return nodeList(this.children as unknown as T[]);
  }

  querySelector<T extends Element>(): T | null {
    return this.preferred as unknown as T | null;
  }

  contains(node: Node | null): boolean {
    return node === (this as unknown as Node) || this.children.includes(node as unknown as FakeElement);
  }
}

function keyboardEvent(key: string, shiftKey = false): KeyboardEvent & {
  preventDefault: ReturnType<typeof vi.fn>;
  stopImmediatePropagation: ReturnType<typeof vi.fn>;
} {
  return {
    key,
    shiftKey,
    preventDefault: vi.fn(),
    stopImmediatePropagation: vi.fn(),
  } as unknown as KeyboardEvent & {
    preventDefault: ReturnType<typeof vi.fn>;
    stopImmediatePropagation: ReturnType<typeof vi.fn>;
  };
}

function modal(document: FakeDocument, focusableCount = 2): {
  dialog: FakeElement;
  controls: FakeElement[];
} {
  const dialog = new FakeElement(document);
  const controls = Array.from({ length: focusableCount }, () => new FakeElement(document));
  dialog.children = controls;
  document.modals.push(dialog);
  return { dialog, controls };
}

describe('modal focus management', () => {
  it('renders a programmatically focusable shared dialog', () => {
    const markup = render(<Dialog title="Confirm" onClose={() => undefined}>Body</Dialog>);
    expect(markup).toContain('role="dialog"');
    expect(markup).toContain('tabindex="-1"');
  });

  it('focuses the preferred control and restores the opener on cleanup', () => {
    const document = new FakeDocument();
    const opener = new FakeElement(document);
    document.activeElement = opener as unknown as Element;
    const { dialog, controls } = modal(document);
    dialog.preferred = controls[1]!;

    const cleanup = activateModalFocus(dialog as unknown as HTMLElement, () => undefined);
    expect(controls[1]!.focusCount).toBe(1);

    cleanup();
    expect(opener.focusCount).toBe(1);
  });

  it('wraps Tab and Shift+Tab inside the active modal', () => {
    const document = new FakeDocument();
    const { dialog, controls } = modal(document);
    const first = controls[0]!;
    const last = controls[1]!;
    activateModalFocus(dialog as unknown as HTMLElement, () => undefined);

    document.activeElement = last as unknown as Element;
    const forward = keyboardEvent('Tab');
    document.dispatch(forward);
    expect(first.focusCount).toBe(2);
    expect(forward.preventDefault).toHaveBeenCalledOnce();

    document.activeElement = first as unknown as Element;
    const backward = keyboardEvent('Tab', true);
    document.dispatch(backward);
    expect(last.focusCount).toBe(1);
    expect(backward.preventDefault).toHaveBeenCalledOnce();

    document.activeElement = new FakeElement(document) as unknown as Element;
    const escapedFocus = keyboardEvent('Tab');
    document.dispatch(escapedFocus);
    expect(first.focusCount).toBe(3);
  });

  it('keeps focus on a modal that has no enabled controls', () => {
    const document = new FakeDocument();
    const { dialog } = modal(document, 0);
    activateModalFocus(dialog as unknown as HTMLElement, () => undefined);
    expect(dialog.focusCount).toBe(1);

    const tab = keyboardEvent('Tab');
    document.dispatch(tab);
    expect(dialog.focusCount).toBe(2);
    expect(tab.preventDefault).toHaveBeenCalledOnce();
  });

  it('closes only the top modal on Escape', () => {
    const document = new FakeDocument();
    const underneath = modal(document, 1);
    const closeUnderneath = vi.fn();
    const closeTop = vi.fn();
    const cleanupUnderneath = activateModalFocus(
      underneath.dialog as unknown as HTMLElement,
      closeUnderneath,
    );
    const top = modal(document, 1);
    const cleanupTop = activateModalFocus(top.dialog as unknown as HTMLElement, closeTop);

    const escape = keyboardEvent('Escape');
    document.dispatch(escape);
    expect(closeUnderneath).not.toHaveBeenCalled();
    expect(closeTop).toHaveBeenCalledOnce();
    expect(escape.preventDefault).toHaveBeenCalledOnce();
    expect(escape.stopImmediatePropagation).toHaveBeenCalledOnce();

    cleanupTop();
    cleanupUnderneath();
  });
});
