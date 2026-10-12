// 构建指纹，由 vite.config.ts 的 define.__BUILD_INFO__ 在构建/开发启动时注入
declare const __BUILD_INFO__: { version: string; hash: string; time: string };

export const BUILD_VERSION: string = __BUILD_INFO__.version;
export const BUILD_HASH: string = __BUILD_INFO__.hash;
export const BUILD_TIME: string = __BUILD_INFO__.time;
