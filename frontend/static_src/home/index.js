// Markets dashboard (Phase C). The base entry runs the live stream and patches
// the watchlist sparkline cards in place; this entry ships the dashboard styles,
// the normalized %-vs-S&P-500 hero graph + market reads, and the watchlist add /
// remove controls.
import "./styles/home.scss";
import { initHero } from "./scripts/hero.js";
import { initWatchlist } from "./scripts/watchlist.js";

initHero();
initWatchlist();
