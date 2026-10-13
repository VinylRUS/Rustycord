/** Simple global context-menu state (one menu at a time). */
export interface MenuItem {
  label: string;
  danger?: boolean;
  disabled?: boolean;
  action: () => void;
}

class MenuStore {
  open = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  show(x: number, y: number, items: MenuItem[]) {
    // keep the menu inside the viewport
    this.open = { x: Math.min(x, window.innerWidth - 220), y: Math.min(y, window.innerHeight - items.length * 36 - 20), items };
  }

  close() {
    this.open = null;
  }

  run(item: MenuItem) {
    this.open = null;
    item.action();
  }
}

export const menu = new MenuStore();
