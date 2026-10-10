/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** "1" runs the mock backend without delays. */
  readonly VITE_MOCK_FAST?: string;
}
