// Filled in by @decky/rollup at build time from plugin.json.
declare module "@decky/manifest" {
  const manifest: {
    name: string;
    author?: string;
    version?: string;
    flags?: string[];
    api_version?: number;
  };
  export default manifest;
}
