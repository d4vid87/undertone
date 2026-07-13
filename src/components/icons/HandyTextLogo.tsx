/* eslint-disable i18next/no-literal-string -- brand wordmark, not UI copy */
import React from "react";

// Undertone wordmark: a small soundwave mark plus lowercase wordmark.
// Kept under the original component name/viewBox so every upstream usage
// (sidebar, onboarding) works untouched.
const HandyTextLogo = ({
  width,
  height,
  className,
}: {
  width?: number;
  height?: number;
  className?: string;
}) => {
  return (
    <svg
      width={width}
      height={height}
      className={className}
      viewBox="0 0 930 328"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      {/* soundwave mark */}
      <rect x="20" y="124" width="26" height="80" rx="13" className="logo-primary" />
      <rect x="62" y="84" width="26" height="160" rx="13" className="logo-primary" />
      <rect x="104" y="44" width="26" height="240" rx="13" className="logo-primary" />
      <rect x="146" y="104" width="26" height="120" rx="13" className="logo-primary" />
      <rect x="188" y="139" width="26" height="50" rx="13" className="logo-primary" />
      {/* wordmark */}
      <text
        x="240"
        y="164"
        dominantBaseline="central"
        fontFamily="ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif"
        fontSize="120"
        fontWeight="800"
        letterSpacing="-2"
        className="logo-primary"
      >
        undertone
      </text>
      <text
        x="242"
        y="252"
        dominantBaseline="central"
        fontFamily="ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif"
        fontSize="34"
        fontWeight="500"
        letterSpacing="6"
        className="logo-stroke"
      >
        LOCAL VOICE, POLISHED TEXT
      </text>
    </svg>
  );
};

export default HandyTextLogo;
