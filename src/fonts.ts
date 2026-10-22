// UI 用的四款拉丁 webfont (与官网 ccrouter.app 一致), 随包打入以便离线可用。
// 只引 latin 子集: 汉字与全角标点在 styles.css 的字体栈里落到系统中文字体,
// 不打包 CJK webfont。字重只列实际用到的, 每个 woff2 约 20–40KB。
import "@fontsource/instrument-sans/latin-400.css";
import "@fontsource/instrument-sans/latin-500.css";
import "@fontsource/instrument-sans/latin-600.css";
import "@fontsource/instrument-sans/latin-700.css";
import "@fontsource/newsreader/latin-500.css";
import "@fontsource/newsreader/latin-600.css";
import "@fontsource/jetbrains-mono/latin-400.css";
import "@fontsource/jetbrains-mono/latin-500.css";
import "@fontsource/jetbrains-mono/latin-600.css";
import "@fontsource/caveat/latin-500.css";
import "@fontsource/caveat/latin-700.css";
