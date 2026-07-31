/**
 * Minimal type declarations for tr46 (UTS-46 / IDNA processing).
 * tr46 does not ship its own TypeScript types.
 */
declare module "tr46" {
  interface ToASCIIOptions {
    checkHyphens?: boolean;
    checkBidi?: boolean;
    checkJoiners?: boolean;
    useSTD3ASCIIRules?: boolean;
    verifyDNSLength?: boolean;
    transitionalProcessing?: boolean;
    ignoreInvalidPunycode?: boolean;
  }

  /**
   * Convert a domain name to its ASCII-Compatible Encoding (ACE) form
   * per UTS-46. Returns `null` if the input cannot be validly encoded.
   */
  function toASCII(domainName: string, options?: ToASCIIOptions): string | null;

  interface ToUnicodeResult {
    domain: string;
    error: boolean;
  }

  function toUnicode(domainName: string, options?: ToASCIIOptions): ToUnicodeResult;

  const _default: {
    toASCII: typeof toASCII;
    toUnicode: typeof toUnicode;
  };

  export default _default;
  export { toASCII, toUnicode };
}
