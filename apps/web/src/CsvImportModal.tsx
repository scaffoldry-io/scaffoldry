import React from "react";
import { ImportWizard, ImportWizardProps } from "./ImportWizard";

export type CsvImportModalProps = ImportWizardProps;

export const CsvImportModal: React.FC<CsvImportModalProps> = (props) => {
  return <ImportWizard {...props} />;
};

export { ImportWizard };
