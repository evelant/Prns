module.exports = {
  preset: "jest-expo",
  testMatch: ["<rootDir>/tests/**/*.test.tsx"],
  clearMocks: true,
  // Expo Router uses the standard name in its optional testing entry point.
  moduleNameMapper: {
    "^@testing-library/react-native$": "@prns/app-testing-library",
  },
};
