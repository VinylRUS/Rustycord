// Единый источник версии для UI: подставляется на сборке из package.json
// (см. define в vite.config.ts), чтобы нигде не захардкоживалась и не
// рассинхронизировалась, как это случилось с «v0.4.0» на странице логина.
declare const __APP_VERSION__: string;

export const APP_VERSION: string =
  typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : 'dev';
