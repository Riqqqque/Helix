import { useCallback, useRef } from 'preact/hooks';

const focusableSelector = 'a[href],button:not(:disabled),input:not(:disabled):not([type=hidden]),select:not(:disabled),textarea:not(:disabled),[tabindex],[contenteditable=true]';

function focusableElements(dialog: HTMLElement): HTMLElement[] {
  return [...dialog.querySelectorAll<HTMLElement>(focusableSelector)].filter((element) =>
    element.tabIndex >= 0 && element.closest('[hidden],[aria-hidden="true"],[inert]') === null
  );
}

function isActiveModal(dialog: HTMLElement): boolean {
  const modals = dialog.ownerDocument.querySelectorAll<HTMLElement>('[role=dialog][aria-modal=true]');
  return modals[modals.length - 1] === dialog;
}

export function activateModalFocus(dialog: HTMLElement, onClose: () => void): () => void {
  const ownerDocument = dialog.ownerDocument;
  const previousFocus = ownerDocument.activeElement as HTMLElement | null;
  const initialElements = focusableElements(dialog);
  const preferred = dialog.querySelector<HTMLElement>('[data-modal-autofocus],[autofocus]');
  (preferred !== null && initialElements.includes(preferred)
    ? preferred
    : initialElements[0] ?? dialog).focus();

  const onKeyDown = (event: KeyboardEvent): void => {
    if (!isActiveModal(dialog)) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopImmediatePropagation();
      onClose();
      return;
    }
    if (event.key !== 'Tab') return;

    const elements = focusableElements(dialog);
    if (elements.length === 0) {
      event.preventDefault();
      dialog.focus();
      return;
    }
    const first = elements[0]!;
    const last = elements[elements.length - 1]!;
    const active = ownerDocument.activeElement;
    if (event.shiftKey && (active === first || !dialog.contains(active))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (active === last || !dialog.contains(active))) {
      event.preventDefault();
      first.focus();
    }
  };

  ownerDocument.addEventListener('keydown', onKeyDown, true);
  return () => {
    ownerDocument.removeEventListener('keydown', onKeyDown, true);
    if (previousFocus?.isConnected === true) previousFocus.focus();
  };
}

export function useModalFocus(onClose: () => void): (element: HTMLElement | null) => void {
  const state = useRef<{ close: () => void; cleanup: (() => void) | null }>({
    close: onClose,
    cleanup: null,
  });
  state.current.close = onClose;
  return useCallback((element: HTMLElement | null): void => {
    state.current.cleanup?.();
    state.current.cleanup = element === null
      ? null
      : activateModalFocus(element, () => state.current.close());
  }, []);
}
