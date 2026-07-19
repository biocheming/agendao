import {
  createContext,
  type PropsWithChildren,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import { detectLocale, translate, type Locale } from "./messages";

type MessageParams = Record<string, string | number>;

interface I18nValue {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: (key: string, params?: MessageParams) => string;
}

const I18nContext = createContext<I18nValue | null>(null);

export function I18nProvider({ children }: PropsWithChildren) {
  const [locale, setLocale] = useState<Locale>(() => detectLocale());

  useEffect(() => {
    if (typeof document === "undefined") return;
    document.documentElement.lang = locale;
  }, [locale]);

  const value = useMemo<I18nValue>(
    () => ({
      locale,
      setLocale,
      t: (key, params) => translate(locale, key, params),
    }),
    [locale],
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

// Components can render outside I18nProvider (e.g. isolated component tests).
// Fall back to translating against the detected locale instead of throwing;
// translate() itself falls back to the English catalog and finally the key.
const fallbackValue: I18nValue = {
  locale: detectLocale(),
  setLocale: () => undefined,
  t: (key, params) => translate(detectLocale(), key, params),
};

export function useI18n() {
  const context = useContext(I18nContext);
  return context ?? fallbackValue;
}
