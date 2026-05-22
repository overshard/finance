// Paper Ledger typefaces:
// - Source Serif 4 — restrained serif, used for headings only
// - Inter — neutral sans, body and UI text
// - JetBrains Mono — tabular figures (the ledger numbers)
import "@fontsource/source-serif-4/400.css";
import "@fontsource/source-serif-4/600.css";
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "@fontsource/jetbrains-mono/700.css";

// styles
import "./styles/base.scss";

// live market stream: patches prices in place and drives the status pill
import { initStream } from "./scripts/stream.js";

initStream();
