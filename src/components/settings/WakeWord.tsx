import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { SettingContainer } from "../ui/SettingContainer";
import { Input } from "../ui/Input";
import { useSettings } from "../../hooks/useSettings";

interface WakeWordProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const WakeWord: React.FC<WakeWordProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const enabled = getSetting("wake_word_enabled") || false;
    const wakeWord = getSetting("wake_word") ?? "";
    const [localWord, setLocalWord] = useState<string | null>(null);

    return (
      <>
        <ToggleSwitch
          checked={enabled}
          onChange={(value) => updateSetting("wake_word_enabled", value)}
          isUpdating={isUpdating("wake_word_enabled")}
          label={t("settings.general.wakeWord.label")}
          description={t("settings.general.wakeWord.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
        />
        {enabled && (
          <SettingContainer
            title={t("settings.general.wakeWord.wordLabel")}
            description={t("settings.general.wakeWord.wordDescription")}
            descriptionMode={descriptionMode}
            grouped={grouped}
          >
            <Input
              type="text"
              variant="compact"
              value={localWord ?? wakeWord}
              onChange={(event) => setLocalWord(event.target.value)}
              onBlur={() => {
                if (localWord !== null && localWord.trim() !== wakeWord) {
                  updateSetting("wake_word", localWord.trim());
                }
                setLocalWord(null);
              }}
              placeholder={t("settings.general.wakeWord.placeholder")}
            />
          </SettingContainer>
        )}
      </>
    );
  },
);
