import { useEffect, useState } from "react";
import { FlatList, Modal, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Asset } from "expo-asset";
import { File, Paths } from "expo-file-system";
import { Button, Text } from "../ui/primitives";
import { Disclosure } from "../ui/Disclosure";
import { useAppPalette } from "../ui/theme";
import type { NoticeDocument } from "./documents";
import { documents } from "./libraryNotices";

function NoticeReader({ document, close }: { document: NoticeDocument; close: () => void }) {
  const palette = useAppPalette();
  const [paragraphs, setParagraphs] = useState<string[]>();
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let active = true;
    async function read() {
      let file: File;
      if ("bundleFile" in document) {
        file = new File(Paths.bundle, document.bundleFile);
      } else {
        const asset = await Asset.fromModule(document.asset()).downloadAsync();
        if (!asset.localUri) throw new Error("Missing packaged notice");
        file = new File(asset.localUri);
      }
      const text = await file.text();
      if (active) setParagraphs(text.split(/\n\s*\n/));
    }
    void read().catch(() => { if (active) setFailed(true); });
    return () => { active = false; };
  }, [document]);
  return <Modal visible onRequestClose={close} presentationStyle="pageSheet">
    <SafeAreaView style={{ flex: 1, padding: 16, gap: 12, backgroundColor: palette.background }}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
        <Text accessibilityRole="header" style={{ flex: 1 }}>{document.title}</Text>
        <Button title="Close" onPress={close} size="compact" />
      </View>
      {failed ? <Text accessibilityRole="alert">Could not open this packaged notice.</Text> :
        paragraphs ? <FlatList data={paragraphs} keyExtractor={(_, index) => String(index)}
          renderItem={({ item }) => <Text selectable style={{ marginBottom: 10 }}>{item}</Text>} /> :
          <Text>Opening notice…</Text>}
    </SafeAreaView>
  </Modal>;
}

export function LicenseNotices() {
  const [selected, setSelected] = useState<NoticeDocument>();
  return <>
    <Disclosure label="Open source licenses and notices">
      {documents.map(document => <Button key={document.title} title={document.title}
        onPress={() => setSelected(document)} tone="quiet" />)}
    </Disclosure>
    {selected && <NoticeReader document={selected} close={() => setSelected(undefined)} />}
  </>;
}
