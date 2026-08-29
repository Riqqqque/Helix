import type { ComponentChildren } from 'preact';
import { Icon } from './icons';
import { useModalFocus } from './modal-focus';

export function Dialog({ title, children, onClose, wide = false, flush = false }: { title: string; children: ComponentChildren; onClose: () => void; wide?: boolean; flush?: boolean }) {
  const modalRef = useModalFocus(onClose);
  return (
    <div class="dialog-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section ref={modalRef} tabIndex={-1} class={`dialog${wide ? ' dialog--wide' : ''}`} role="dialog" aria-modal="true" aria-labelledby="dialog-title">
        <header><h2 id="dialog-title">{title}</h2><button class="icon-button" type="button" onClick={onClose} aria-label="Close dialog"><Icon name="close" /></button></header>
        {flush ? children : <div class="dialog-body">{children}</div>}
      </section>
    </div>
  );
}
