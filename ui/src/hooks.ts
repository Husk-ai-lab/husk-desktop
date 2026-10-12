import { useCallback, useMemo, useState } from "react";
import { detectLang, makeT, saveLang, type Lang } from "./i18n";

export function useLang() {
  const [lang, setLangState] = useState<Lang>(detectLang());
  const t = useMemo(() => makeT(lang), [lang]);
  const setLang = useCallback((l: Lang) => {
    saveLang(l);
    setLangState(l);
  }, []);
  return { lang, setLang, t };
}
