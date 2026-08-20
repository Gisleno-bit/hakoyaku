//! Limpieza del texto que devuelve el OCR.
//!
//! El motor de OCR de Windows separa en "palabras" tambien los idiomas que no
//! usan espacios (japones, chino, coreano). El resultado crudo de una linea como
//! `積雪の様にみっしりと` suele llegar como `積 雪 の 様 に みっしり と`.
//!
//! Todo lo de este modulo son funciones puras: no tocan pantalla, ni red, ni
//! sistema operativo. Por eso se pueden testear en cualquier plataforma.

/// Caracteres que en su idioma no llevan espacios entre si.
pub fn is_scriptio_continua(c: char) -> bool {
    let u = c as u32;
    matches!(u,
        0x3000..=0x303F   // puntuacion CJK: 、。「」・…
        | 0x3040..=0x309F // hiragana
        | 0x30A0..=0x30FF // katakana
        | 0x31F0..=0x31FF // katakana fonetico extendido
        | 0x3400..=0x4DBF // kanji ext. A
        | 0x4E00..=0x9FFF // kanji comunes
        | 0xF900..=0xFAFF // kanji de compatibilidad
        | 0xFF00..=0xFFEF // formas half/fullwidth
        | 0x1100..=0x11FF // jamo coreano
        | 0x3130..=0x318F // jamo compatible
        | 0xAC00..=0xD7AF // silabas hangul
    )
}

/// `true` si el texto contiene al menos un caracter CJK.
pub fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        let u = c as u32;
        matches!(u, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF)
    })
}

/// Quita los espacios que el OCR mete entre caracteres CJK, pero respeta los
/// espacios legitimos entre palabras latinas.
pub fn join_intra_line(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if c == ' ' || c == '\u{3000}' || c == '\t' {
            // Buscamos el siguiente caracter no-espacio.
            let mut j = i;
            while j < chars.len() && (chars[j] == ' ' || chars[j] == '\u{3000}' || chars[j] == '\t')
            {
                j += 1;
            }
            let prev = out.chars().next_back();
            let next = chars.get(j).copied();

            let drop_space = match (prev, next) {
                (None, _) => true, // espacio inicial
                (_, None) => true, // espacio final
                (Some(p), Some(n)) => is_scriptio_continua(p) || is_scriptio_continua(n),
            };

            if !drop_space {
                out.push(' ');
            }
            i = j;
            continue;
        }

        out.push(c);
        i += 1;
    }

    out
}

/// Une varias lineas de dialogo en un solo bloque.
///
/// Entre dos lineas japonesas no se mete nada; entre texto latino se mete un
/// espacio. Se conserva el salto de parrafo cuando la linea anterior termina en
/// puntuacion de cierre japonesa.
pub fn join_lines(lines: &[String]) -> String {
    let mut out = String::new();

    for raw in lines {
        let line = join_intra_line(raw);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if out.is_empty() {
            out.push_str(line);
            continue;
        }

        let prev = out.chars().next_back().unwrap_or(' ');
        let next = line.chars().next().unwrap_or(' ');

        if matches!(prev, '。' | '！' | '？' | '」' | '』') {
            out.push('\n');
        } else if !is_scriptio_continua(prev) && !is_scriptio_continua(next) {
            out.push(' ');
        }
        out.push_str(line);
    }

    out
}

/// Pasada final: colapsa espacios repetidos y recorta.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_space = false;

    for c in s.chars() {
        let is_space = c == ' ' || c == '\u{3000}' || c == '\t';
        if is_space {
            if !last_was_space {
                out.push(' ');
            }
            last_was_space = true;
        } else {
            out.push(c);
            last_was_space = false;
        }
    }

    out.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

/// Pipeline completo: lineas crudas del OCR -> texto limpio listo para traducir.
pub fn clean_ocr_lines(lines: &[String]) -> String {
    normalize(&join_lines(lines))
}

/// Descarta capturas que casi seguro son ruido (marco vacio, un icono, etc.).
///
/// `min_chars` cuenta caracteres, no bytes. `require_cjk` sirve cuando sabemos
/// que el idioma de origen es japones y no queremos gastar cuota de API en
/// numeritos de HUD.
pub fn is_worth_translating(text: &str, min_chars: usize, require_cjk: bool) -> bool {
    let visible = text.chars().filter(|c| !c.is_whitespace()).count();
    if visible < min_chars {
        return false;
    }
    if require_cjk && !has_cjk(text) {
        return false;
    }
    true
}

/// Deshace las entidades HTML que devuelven algunas APIs (Google, sobre todo)
/// incluso cuando pides `format=text`.
pub fn unescape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == '&' {
            let rest: String = bytes[i..].iter().take(8).collect();
            let replacement = if rest.starts_with("&amp;") {
                Some(("&", 5))
            } else if rest.starts_with("&lt;") {
                Some(("<", 4))
            } else if rest.starts_with("&gt;") {
                Some((">", 4))
            } else if rest.starts_with("&quot;") {
                Some(("\"", 6))
            } else if rest.starts_with("&#39;") {
                Some(("'", 5))
            } else if rest.starts_with("&nbsp;") {
                Some((" ", 6))
            } else {
                None
            };

            if let Some((rep, len)) = replacement {
                out.push_str(rep);
                i += len;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quita_espacios_entre_kanji_y_kana() {
        let entrada = "積 雪 の 様 に みっしり と";
        assert_eq!(join_intra_line(entrada), "積雪の様にみっしりと");
    }

    #[test]
    fn conserva_espacios_entre_palabras_latinas() {
        assert_eq!(join_intra_line("GAME OVER  press start"), "GAME OVER press start");
    }

    #[test]
    fn mezcla_latino_y_japones() {
        // Sin espacio junto al CJK, con espacio entre las dos palabras latinas.
        assert_eq!(join_intra_line("HP が 100 to 50"), "HPが100 to 50");
    }

    #[test]
    fn une_dos_lineas_de_dialogo() {
        let lineas = vec![
            "積 雪 の 様 に みっしり と".to_string(),
            "白 い 絨 毯 が 敷 か れ た 奇 妙 な 空 間 に 出 る 。".to_string(),
        ];
        assert_eq!(
            clean_ocr_lines(&lineas),
            "積雪の様にみっしりと白い絨毯が敷かれた奇妙な空間に出る。"
        );
    }

    #[test]
    fn corta_parrafo_tras_punto_japones() {
        let lineas = vec!["こんにちは。".to_string(), "元気ですか。".to_string()];
        assert_eq!(clean_ocr_lines(&lineas), "こんにちは。\n元気ですか。");
    }

    #[test]
    fn descarta_lineas_vacias() {
        let lineas = vec!["".to_string(), "   ".to_string(), "テスト".to_string()];
        assert_eq!(clean_ocr_lines(&lineas), "テスト");
    }

    #[test]
    fn detecta_cjk() {
        assert!(has_cjk("空間"));
        assert!(has_cjk("abc あ"));
        assert!(!has_cjk("Press START"));
        assert!(!has_cjk("123 - 456"));
    }

    #[test]
    fn con_el_minimo_en_dos_pasan_los_botones_de_menu() {
        // はい, 見る, 戻る: opciones de dos caracteres que hay que traducir.
        assert!(is_worth_translating("はい", 2, true));
        assert!(is_worth_translating("見る", 2, true));
        assert!(is_worth_translating("いいえ", 2, true));
        // Pero un caracter suelto sigue siendo ruido.
        assert!(!is_worth_translating("・", 2, true));
    }

    #[test]
    fn filtra_ruido_corto() {
        assert!(!is_worth_translating("あ", 3, true));
        assert!(is_worth_translating("こんにちは", 3, true));
        assert!(!is_worth_translating("100/250", 3, true));
        assert!(is_worth_translating("100/250", 3, false));
    }

    #[test]
    fn los_espacios_no_cuentan_para_el_minimo() {
        assert!(!is_worth_translating("あ  い", 3, true));
    }

    #[test]
    fn normaliza_espacios_repetidos() {
        assert_eq!(normalize("  hola    mundo  "), "hola mundo");
        assert_eq!(normalize("a\n\n\nb"), "a\nb");
    }

    #[test]
    fn desescapa_html() {
        assert_eq!(unescape_html("¿Qu&#39;e tal?"), "¿Qu'e tal?");
        assert_eq!(unescape_html("a &amp; b &lt;c&gt;"), "a & b <c>");
        assert_eq!(unescape_html("sin entidades"), "sin entidades");
    }
}
