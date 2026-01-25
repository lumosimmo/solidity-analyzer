import { COMMANDS } from "./commandIds";
import type { StatusBarClickAction, StatusBarShow } from "./config";

export const STATUS_BAR_LANGUAGE_ID = "solidity";

export function shouldShowStatusBar(show: StatusBarShow, languageId?: string): boolean {
    if (show === "always") {
        return true;
    }
    if (show === "never") {
        return false;
    }
    return languageId === STATUS_BAR_LANGUAGE_ID;
}

export function statusBarCommand(action: StatusBarClickAction): string {
    return action === "restartServer" ? COMMANDS.restartServer : COMMANDS.openLogs;
}
