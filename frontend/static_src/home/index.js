// Markets dashboard. The live stream runs from the base entry and patches the
// sparkline cards in place; this entry ships the dashboard styles and the
// Phase-7 live patcher for the hero verdict + breadth band.
import "./styles/home.scss";
import { initSummary } from "./scripts/home.js";

initSummary();
