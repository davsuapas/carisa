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
	"io/ioutil"
	"os"

	"encoding/json"

	"github.com/carisa/pkg/strings"
	"github.com/pkg/errors"
)

// Read reads the config from file or environment variable.
// The ref is the file name or the environment variable depending
// of the file parameter.
// The result read is assigned to the parameter confg
func Read(file bool, ref string, confg interface{}) error {
	var res []byte

	if file {
		var err error
		res, err = ioutil.ReadFile(ref)
		if err != nil {
			return errors.Wrap(err, strings.Concat("cannot read the configuration file. Ref: ", ref))
		}
	} else {
		res = []byte(os.Getenv(ref))
	}

	if len(res) == 0 {
		return nil
	}

	if err := json.Unmarshal(res, &confg); err != nil {
		return errors.Wrap(err,
			strings.Concat("cannot unmarshal the configuration. Ref: ", ref, ", Source: ", string(res)))
	}

	return nil
}
