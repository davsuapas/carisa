/*
 *   Copyright (c) 2021 CARISA
 *   All rights reserved.

 *   Licensed under the Apache License, Version 2.0 (the "License");
 *   you may not use this file except in compliance with the License.
 *   You may obtain a copy of the License at

 *   http://www.apache.org/licenses/LICENSE-2.0

 *   Unless required by applicable law or agreed to in writing, software
 *   distributed under the License is distributed on an "AS IS" BASIS,
 *   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 *   See the License for the specific language governing permissions and
 *   limitations under the License.
 */

package config

import (
	"github.com/carisa/pkg/strings"
	"go.uber.org/zap"
)

// NewLogger creates the zap logger
func NewLogger(config Zap) *zap.Logger {
	var logc zap.Config
	if config.Development {
		logc = zap.NewDevelopmentConfig()
	} else {
		logc = zap.NewProductionConfig()
	}
	logc.Level = zap.NewAtomicLevelAt(config.Level)
	logc.Encoding = config.Encoding
	log, err := logc.Build()
	if err != nil {
		panic(strings.Concat("Error creating zap logger. Error: ", err.Error()))
	}
	return log
}
